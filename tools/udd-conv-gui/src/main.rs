use eframe::egui;
use color_eyre::eyre;

mod app;
mod models;
mod logic;
mod ui;

use app::UddConvApp;
use models::{LogLevel, Tab};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 800.0])
            .with_min_inner_size([900.0, 700.0])
            .with_title("UODynamapper Asset Converter"),
        ..Default::default()
    };

    eframe::run_native(
        "uddconv_gui",
        native_options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            let app = UddConvApp::new(cc);
            Ok(Box::new(app))
        }),
    ).map_err(|e| eyre::eyre!("eframe error: {}", e))?;

    Ok(())
}

impl eframe::App for UddConvApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_visuals(egui::Visuals::dark());
        let settings_before = self.settings.clone();

        // External preview window (for big textures)
        self.ui_preview_window(ctx);
        self.ui_upscale_preview_window(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.add_space(5.0);
                ui.heading("UODynamapper Asset Converter");
                ui.add_space(15.0);

                // Tab navigation
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.current_tab, Tab::Sources, "📁 Sources");
                    ui.selectable_value(&mut self.current_tab, Tab::Assets, "🎨 Assets");
                    ui.selectable_value(&mut self.current_tab, Tab::World, "🌍 World");
                    ui.selectable_value(&mut self.current_tab, Tab::Tools, "🛠 Tools");
                });

                ui.add_space(5.0);
                ui.separator();
                ui.add_space(15.0);

                let available_height = ui.available_height();
                let log_panel_height = 200.0;
                let content_height = (available_height - log_panel_height - 10.0).max(160.0);

                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), content_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("main_scroll")
                            .show(ui, |ui| match self.current_tab {
                                Tab::Sources => self.ui_sources(ui),
                                Tab::Assets => self.ui_assets(ui),
                                Tab::World => self.ui_world(ui),
                                Tab::Tools => self.ui_tools(ui),
                            });
                    },
                );

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);

                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), log_panel_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.heading("Logs");
                            if ui.button("Clear").clicked() {
                                if let Ok(mut logs) = self.logs.lock() {
                                    logs.clear();
                                }
                            }
                            if self.is_busy() {
                                ui.spinner();
                                ui.label("Processing...");
                            }
                        });

                        ui.add_space(5.0);
                        let text_edit_id = ui.make_persistent_id("log_view");
                        egui::ScrollArea::vertical()
                            .id_salt(text_edit_id)
                            .auto_shrink([false, false])
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());

                                let Ok(logs) = self.logs.lock() else {
                                    return;
                                };

                                for log in logs.iter() {
                                    let color = match log.level {
                                        LogLevel::Info => egui::Color32::from_gray(200),
                                        LogLevel::Success => egui::Color32::from_rgb(100, 255, 100),
                                        // LogLevel::Warning => egui::Color32::from_rgb(255, 200, 0),
                                        LogLevel::Error => egui::Color32::from_rgb(255, 100, 100),
                                    };
                                    ui.colored_label(color, &log.text);
                                }
                            });
                    },
                );
            });
        });

        // Keep repainting while a background task is running
        if self.is_busy() {
            ctx.request_repaint();
        }

        if self.settings != settings_before {
            self.persist_settings();
        }
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.persist_settings();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.persist_settings();
    }
}
