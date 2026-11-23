use eframe::egui;
use color_eyre::eyre;
use uot_gui::{run_native_with_setup, show_log_text, NativeWindow};

mod app;
mod models;
mod logic;
mod ui;

use app::UddConvApp;
use models::{LogLevel, Tab};
use ui::layout::section_heading;

const APP_VIEWPORT_WIDTH: f32 = 1100.0;
const APP_VIEWPORT_HEIGHT: f32 = 800.0;
const APP_MIN_VIEWPORT_WIDTH: f32 = 900.0;
const APP_MIN_VIEWPORT_HEIGHT: f32 = 700.0;
const LOG_PANEL_DEFAULT_HEIGHT: f32 = 250.0;
const LOG_PANEL_MIN_HEIGHT: f32 = 80.0;
const MIN_CONTENT_HEIGHT: f32 = 160.0;

fn main() -> eyre::Result<()> {
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

        egui::Panel::top("navigation").show(ui, |ui| {
            ui.add_space(5.0);
            ui.heading("UODynamapper Asset Converter");
            ui.add_space(15.0);

            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.current_tab, Tab::QuickConvert, "Quick Convert");
                ui.selectable_value(&mut self.current_tab, Tab::Sources, "📁 Sources");
                ui.selectable_value(&mut self.current_tab, Tab::Assets, "🎨 Assets");
                ui.selectable_value(&mut self.current_tab, Tab::World, "🌍 World");
                ui.selectable_value(&mut self.current_tab, Tab::Tools, "🛠 Tools");
            });
        });

        egui::Panel::bottom("log_panel")
            .resizable(true)
            .default_size(LOG_PANEL_DEFAULT_HEIGHT)
            .min_size(LOG_PANEL_MIN_HEIGHT)
            .max_size((ui.available_height() - MIN_CONTENT_HEIGHT).max(LOG_PANEL_MIN_HEIGHT))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(section_heading("Logs"));
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

                let logs = self.logs.lock().map(|logs| logs.clone()).unwrap_or_default();
                let lines = logs.iter().map(|log| {
                    let color = match log.level {
                        LogLevel::Info => egui::Color32::from_gray(200),
                        LogLevel::Success => egui::Color32::from_rgb(100, 255, 100),
                        LogLevel::Warning => egui::Color32::from_rgb(255, 200, 0),
                        LogLevel::Error => egui::Color32::from_rgb(255, 100, 100),
                    };
                    (log.text.as_str(), Some(color))
                });
                show_log_text(ui, "logs", lines, egui::TextStyle::Body);
            });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("main_scroll")
                .show(ui, |ui| match self.current_tab {
                    Tab::QuickConvert => self.ui_quick_convert(ui),
                    Tab::Sources => self.ui_sources(ui),
                    Tab::Assets => self.ui_assets(ui),
                    Tab::World => self.ui_world(ui),
                    Tab::Tools => self.ui_tools(ui),
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
