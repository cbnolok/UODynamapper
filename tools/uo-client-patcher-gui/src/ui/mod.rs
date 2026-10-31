//! The window: top bar, sections in a scroll area, log panel.

mod data_check;
mod executable;
mod facets;
mod log_panel;
mod login;
mod preview;
mod seam;

use eframe::egui;

use crate::app::{PatcherApp, Section};

pub const ERROR_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 110, 110);
pub const WARNING_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 200, 0);
pub const SUCCESS_COLOR: egui::Color32 = egui::Color32::from_rgb(110, 230, 110);
pub const MUTED_COLOR: egui::Color32 = egui::Color32::from_gray(150);
const SECTION_SPACING: f32 = 6.0;
pub const ITEM_SPACING: f32 = 4.0;
const GRID_SPACING: [f32; 2] = [12.0, 4.0];
const LOG_PANEL_DEFAULT_HEIGHT: f32 = 150.0;
const LOG_PANEL_MIN_HEIGHT: f32 = 60.0;

pub fn error_label(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.add(egui::Label::new(egui::RichText::new(text.into()).color(ERROR_COLOR)).wrap());
}

pub fn warning_label(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.add(egui::Label::new(egui::RichText::new(text.into()).color(WARNING_COLOR)).wrap());
}

pub fn muted_label(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.add(egui::Label::new(egui::RichText::new(text.into()).color(MUTED_COLOR)).wrap());
}

/// A fixed-width, left-aligned cell of a row.
pub fn cell(ui: &mut egui::Ui, width: f32, add_contents: impl FnOnce(&mut egui::Ui) -> egui::Response) -> egui::Response {
    let height = ui.spacing().interact_size.y;
    ui.allocate_ui_with_layout(egui::vec2(width, height), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(width);
        add_contents(ui)
    })
    .inner
}

/// A label/value grid of two columns.
pub fn info_grid(ui: &mut egui::Ui, id: &str, rows: &[(&str, String)]) {
    egui::Grid::new(id).num_columns(2).spacing(GRID_SPACING).show(ui, |ui| {
        for (label, value) in rows {
            ui.label(egui::RichText::new(*label).strong());
            ui.add(egui::Label::new(value.as_str()).wrap());
            ui.end_row();
        }
    });
}

impl PatcherApp {
    pub fn ui_window(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.update(&ctx);

        egui::Panel::top("top_bar").show(ui, |ui| self.ui_top_bar(ui));
        egui::Panel::bottom("log_panel")
            .resizable(true)
            .default_size(LOG_PANEL_DEFAULT_HEIGHT)
            .min_size(LOG_PANEL_MIN_HEIGHT)
            .show(ui, |ui| log_panel::ui_log_panel(ui, &self.logs));
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("sections").auto_shrink([false, false]).show(ui, |ui| {
                for section in Section::ALL {
                    egui::CollapsingHeader::new(egui::RichText::new(section.title()).heading())
                        .id_salt(section.title())
                        .default_open(section.default_open())
                        .show(ui, |ui| self.ui_section(ui, section));
                    ui.add_space(SECTION_SPACING);
                }
            });
        });
    }

    fn ui_top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("📂 Open executable…").clicked() {
                self.open_dialog();
            }
            let hovering = ui.ctx().input(|input| !input.raw.hovered_files.is_empty());
            match &self.session {
                _ if hovering => {
                    ui.label(egui::RichText::new("Drop to open").strong().color(SUCCESS_COLOR));
                }
                Some(session) => {
                    ui.label(egui::RichText::new(session.opened.path.to_string_lossy()).strong());
                    ui.label(egui::RichText::new(session.opened.inspection.family.label()).color(SUCCESS_COLOR));
                }
                None => muted_label(ui, "or drop a client executable on the window"),
            }
        });
    }

    fn ui_section(&mut self, ui: &mut egui::Ui, section: Section) {
        if section == Section::LoginServer {
            // It needs no executable yet.
            login::ui_login(ui);
            return;
        }
        if section == Section::Executable {
            if let Some(error) = &self.open_error {
                error_label(ui, format!("Cannot open: {error}"));
            }
        }
        let Some(session) = &mut self.session else {
            if section == Section::Executable {
                muted_label(ui, "No executable open.");
            }
            return;
        };
        match section {
            Section::Executable => executable::ui_executable(ui, session),
            Section::Facets => facets::ui_facets(ui, session, &mut self.settings.hide_file_ids),
            Section::Seam => seam::ui_seam(ui, session),
            Section::DataCheck => {
                if data_check::ui_data_check(ui, session, &mut self.settings.last_data_dir) {
                    session.start_data_check(ui.ctx(), &self.logs);
                }
            }
            Section::PreviewWrite => {
                if let Some(patch) = preview::ui_preview(ui, session) {
                    self.save_as(&patch);
                }
            }
            Section::LoginServer => {}
        }
    }
}
