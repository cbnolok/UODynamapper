use eframe::egui;
use color_eyre::eyre;
use gui_shared::{run_native_with_setup, NativeWindow};

mod app;
mod models;
mod logic;
mod ui;

use app::UddConvApp;
use models::{LogLevel, Tab};

const APP_VIEWPORT_WIDTH: f32 = 1100.0;
const APP_VIEWPORT_HEIGHT: f32 = 800.0;
const APP_MIN_VIEWPORT_WIDTH: f32 = 900.0;
const APP_MIN_VIEWPORT_HEIGHT: f32 = 700.0;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    color_eyre::install()?;

    run_native_with_setup(
        "uddconv_gui",
        NativeWindow {
            title: "UODynamapper Asset Converter",
            width: APP_VIEWPORT_WIDTH,
            height: APP_VIEWPORT_HEIGHT,
            min_width: APP_MIN_VIEWPORT_WIDTH,
            min_height: APP_MIN_VIEWPORT_HEIGHT,
        },
        false,
        |cc| Ok(Box::new(UddConvApp::new(cc))),
    ).map_err(|e| eyre::eyre!("eframe error: {}", e))?;

    Ok(())
}

impl eframe::App for UddConvApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ctx.set_visuals(egui::Visuals::dark());
        let settings_before = self.settings.clone();

        // External preview window (for big textures)
        self.ui_preview_window(&ctx);
        self.ui_upscale_preview_window(&ctx);

        egui::CentralPanel::default().show_inside(ui, |ui| {
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
                        let log_scroll_height = ui.available_height();
                        let text_edit_id = ui.make_persistent_id("log_view");
                        egui::ScrollArea::vertical()
                            .id_salt(text_edit_id)
                            .max_height(log_scroll_height)
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
                                        LogLevel::Warning => egui::Color32::from_rgb(255, 200, 0),
                                        LogLevel::Error => egui::Color32::from_rgb(255, 100, 100),
                                    };
                                    ui.colored_label(color, &log.text);
                                }
                                ui.add_space(6.0);
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
