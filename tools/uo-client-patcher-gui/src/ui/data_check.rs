//! Section 4: the facet data of a client folder against the edited sizes, before writing.

use std::path::PathBuf;

use eframe::egui;
use uocf::client::facet_data::FacetDataStatus;
use uocf::client::facet_dimensions::ClientFamily;

use super::{error_label, muted_label, warning_label, ERROR_COLOR, ITEM_SPACING, MUTED_COLOR, SUCCESS_COLOR};
use crate::app::Session;

const SOURCE_WIDTH: f32 = 260.0;
const STATUS_WIDTH: f32 = 70.0;

/// Draws the section; returns true when the check is asked for.
pub fn ui_data_check(ui: &mut egui::Ui, session: &mut Session, last_data_dir: &mut Option<PathBuf>) -> bool {
    let what = match session.edits.family {
        ClientFamily::Classic => "the map, staidx and statics files of both table copies",
        ClientFamily::Enhanced => "facetN.uop / facetNx.uop sectors and data.map, facets.uop, facetNN.dds",
        ClientFamily::KingdomReborn => "facetN.uop / facetNx.uop sectors and data.map, facetNN.dds",
    };
    muted_label(ui, format!("Checks {what} against the sizes in the Facets section. Only reads."));
    let mut run = false;
    ui.horizontal(|ui| {
        ui.label("Client data folder:");
        ui.label(egui::RichText::new(session.data_dir.to_string_lossy()).monospace());
        if ui.button("Choose…").clicked()
            && let Some(dir) = uot_gui::file_dialog().set_directory(&session.data_dir).pick_folder()
        {
            *last_data_dir = Some(dir.clone());
            session.data_dir = dir;
        }
        let exe_folder = session.opened.folder();
        if ui.add_enabled(session.data_dir != exe_folder, egui::Button::new("Executable's folder")).clicked() {
            session.data_dir = exe_folder;
        }
        if let Some(dir) = last_data_dir.clone().filter(|dir| *dir != session.data_dir && dir.is_dir())
            && ui.button("Last chosen").on_hover_text(dir.to_string_lossy()).clicked()
        {
            session.data_dir = dir;
        }
    });
    let dimensions = session.edits.edited_dimensions();
    ui.horizontal(|ui| {
        let running = session.running_check.is_some();
        run = ui.add_enabled(!running && dimensions.is_ok(), egui::Button::new("Check against the edited sizes")).clicked();
        if running {
            ui.spinner();
            ui.label("Checking…");
        }
    });
    if let Err(error) = &dimensions {
        error_label(ui, error.as_str());
    }
    let Some(check) = &session.data_check else {
        return run;
    };
    ui.add_space(ITEM_SPACING);
    let current = dimensions.as_ref().is_ok_and(|dimensions| check.is_for(dimensions, &session.data_dir));
    if !current {
        warning_label(ui, "The sizes or the folder changed since this check: run it again.");
    }
    let reports = match &check.reports {
        Ok(reports) => reports,
        Err(error) => {
            error_label(ui, format!("The check failed: {error}"));
            return run;
        }
    };
    let counts = check.counts();
    let summary = format!("{} match, {} mismatch, {} absent", counts.ok, counts.mismatch, counts.absent);
    ui.colored_label(if counts.mismatch == 0 { SUCCESS_COLOR } else { ERROR_COLOR }, summary);
    let names = &session.edits.rows;
    for report in reports {
        let name = names.get(usize::from(report.facet)).map(|row| row.name.as_str()).unwrap_or_default();
        let geometry = report.geometry;
        let mismatches = report.mismatches().count();
        let title = format!(
            "facet {} {name} {} x {}: {}",
            report.facet,
            geometry.width,
            geometry.height,
            if mismatches == 0 { "ok".to_owned() } else { format!("{mismatches} mismatch") }
        );
        let color = if mismatches == 0 { SUCCESS_COLOR } else { ERROR_COLOR };
        egui::CollapsingHeader::new(egui::RichText::new(title).color(color))
            .id_salt(("data_check_facet", report.facet))
            .default_open(mismatches > 0)
            .show(ui, |ui| {
                for item in &report.items {
                    let (status, color) = match item.status {
                        FacetDataStatus::Ok => ("ok", SUCCESS_COLOR),
                        FacetDataStatus::Mismatch => ("MISMATCH", ERROR_COLOR),
                        FacetDataStatus::Absent => ("absent", MUTED_COLOR),
                    };
                    ui.horizontal(|ui| {
                        super::cell(ui, STATUS_WIDTH, |ui| ui.colored_label(color, status));
                        super::cell(ui, SOURCE_WIDTH, |ui| ui.monospace(&item.source));
                        ui.label(&item.detail);
                    });
                }
            });
    }
    run
}
