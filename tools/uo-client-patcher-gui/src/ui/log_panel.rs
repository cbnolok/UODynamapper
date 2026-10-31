//! The log panel at the bottom of the window.

use eframe::egui;

use crate::panel_logger::{LogLevel, Logs};

const INFO_COLOR: egui::Color32 = egui::Color32::from_gray(200);
const SUCCESS_COLOR: egui::Color32 = egui::Color32::from_rgb(100, 255, 100);
const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 200, 0);
const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 100, 100);

pub fn ui_log_panel(ui: &mut egui::Ui, logs: &Logs) {
    ui.horizontal(|ui| {
        ui.heading("Logs");
        if ui.button("Clear").clicked() {
            logs.clear();
        }
    });
    ui.separator();
    egui::ScrollArea::vertical().id_salt("logs").auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        for message in logs.snapshot() {
            let color = match message.level {
                LogLevel::Info => INFO_COLOR,
                LogLevel::Success => SUCCESS_COLOR,
                LogLevel::Warning => WARNING_COLOR,
                LogLevel::Error => ERROR_COLOR,
            };
            ui.add(egui::Label::new(egui::RichText::new(message.text).color(color).monospace()).wrap());
        }
    });
}
