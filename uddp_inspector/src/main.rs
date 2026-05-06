use color_eyre::eyre;
use eframe::egui;

mod app;
mod models;
mod utils;
mod logic;
mod ui;

use app::InspectorApp;
use models::ViewMode;
use utils::{format_size, open_package_dialog};

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
                    if let Some(path) = open_package_dialog() {
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
                self.render_table(ctx, ui);
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
}

impl InspectorApp {
    fn ui_left_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Package Info");

        let Some(u_reader) = &self.package else {
            ui.label("No package loaded.");
            return;
        };

        let header = u_reader.header();
        egui::Grid::new("header_grid").show(ui, |ui| {
            ui.label("Version:");
            ui.label(format!("{}.{}", header.version_major, header.version_minor));
            ui.end_row();
            ui.label("Lookup Mode:");
            ui.label(format!("{:?}", u_reader.lookup_mode()));
            ui.end_row();
            ui.label("Files:");
            ui.label(header.file_count.to_string());
            ui.end_row();
        });

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        // View Mode toggle (only if virtual entries were detected)
        if !self.virtual_entries.is_empty() {
            ui.heading("View Mode");
            ui.horizontal(|ui| {
                let mut virtual_mode = self.view_mode == ViewMode::Virtual;
                if ui
                    .checkbox(&mut virtual_mode, "Virtual View (per entry)")
                    .changed()
                {
                    self.view_mode = if virtual_mode {
                        ViewMode::Virtual
                    } else {
                        ViewMode::Package
                    };
                    self.selected_idx = None;
                    self.selected_virtual_idx = None;
                    self.preview_text = None;
                    self.preview_texture = None;
                }
            });
            ui.add_space(10.0);
        }

        ui.label("Filter ID / Hash / Name:");
        let resp = ui.text_edit_singleline(&mut self.filter);
        if self.focus_filter {
            resp.request_focus();
            self.focus_filter = false;
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(10.0);

        ui.heading("Dictionaries");
        let dicts = u_reader.dictionary_records();
        if dicts.is_empty() {
            ui.label("No dictionaries.");
            return;
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("dict_grid").show(ui, |ui| {
                for (dtype, codec, size) in dicts {
                    ui.label(format!("T{}:", dtype));
                    ui.label(format!("{:?} ({})", codec, crate::utils::format_size(size as u64)));
                    ui.end_row();
                }
            });
        });
    }

    fn ui_texture_viewer_window(&mut self, ctx: &egui::Context) {
        let viewer_data = if let Some((texture, _size, label)) = self.image_for_window() {
            Some((texture.clone(), label.to_string()))
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
                    let Some((texture, label)) = &viewer_data else {
                        ui.label("No image available for the current selection.");
                        return;
                    };

                    ui.label(label.as_str());
                    ui.separator();
                    egui::ScrollArea::both().show(ui, |ui| {
                        ui.image(texture);
                    });
                });
            },
        );

        if close_requested {
            self.image_window_open = false;
        }
    }
}
