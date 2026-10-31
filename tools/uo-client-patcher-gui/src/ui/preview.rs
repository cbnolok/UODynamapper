//! Section 5: the dry run of every edit as one patch, then "Save as".

use eframe::egui;
use uocf::client_exe::patch::ExePatch;

use super::{error_label, muted_label, warning_label, ITEM_SPACING, SUCCESS_COLOR};
use crate::app::{Session, WriteOutcome};
use crate::logic::preview::{change_rows, Preview};

/// The change list scrolls past this height.
const CHANGES_MAX_HEIGHT: f32 = 260.0;
const CHANGE_GRID_SPACING: [f32; 2] = [14.0, 2.0];

/// Draws the section; returns the patch to write when "Save as" is clicked.
pub fn ui_preview(ui: &mut egui::Ui, session: &mut Session) -> Option<ExePatch> {
    let blockers = session.opened.blockers();
    for blocker in &blockers {
        error_label(ui, format!("Cannot be patched: {blocker}"));
    }
    let preview = session.preview.get(&session.opened, &session.edits);
    let mut save = None;
    match preview {
        Preview::Empty => muted_label(ui, "Nothing edited: the facets and the seam are as in the file."),
        Preview::Invalid(errors) => {
            ui.label("Fix these before writing:");
            for error in errors {
                error_label(ui, error.as_str());
            }
        }
        Preview::Refused(error) => error_label(ui, format!("The patch is refused: {error}")),
        Preview::Planned(plan) => {
            let patch = &plan.patch;
            ui.label(egui::RichText::new(format!("{} edits, {} byte changes:", patch.edits.len(), plan.changes.len())).strong());
            for (index, edit) in patch.edits.iter().enumerate() {
                ui.label(format!("  {}. {}", index + 1, edit.describe()));
            }
            for warning in &plan.warnings {
                warning_label(ui, warning.text.as_str());
            }
            ui.add_space(ITEM_SPACING);
            egui::ScrollArea::both().id_salt("byte_changes").max_height(CHANGES_MAX_HEIGHT).show(ui, |ui| {
                egui::Grid::new("byte_changes_grid").striped(true).num_columns(6).spacing(CHANGE_GRID_SPACING).show(
                    ui,
                    |ui| {
                        for title in ["File offset", "VA", "Field", "Old", "New", "Cause"] {
                            ui.label(egui::RichText::new(title).strong());
                        }
                        ui.end_row();
                        for row in change_rows(&plan.changes, patch) {
                            ui.monospace(row.offset);
                            ui.monospace(row.va);
                            ui.label(row.field);
                            ui.monospace(row.old);
                            ui.monospace(row.new);
                            ui.label(row.cause);
                            ui.end_row();
                        }
                    },
                );
            });
            ui.add_space(ITEM_SPACING);
            ui.horizontal(|ui| {
                let clicked = ui
                    .add_enabled(blockers.is_empty(), egui::Button::new("💾 Save as…"))
                    .on_hover_text("Writes a patched copy and its manifest; the input file is never changed")
                    .clicked();
                if clicked {
                    save = Some(patch.clone());
                }
                muted_label(ui, "Outside the input executable's folder, to a new file name.");
            });
        }
    }
    match &session.last_write {
        Some(WriteOutcome::Written { copy, manifest }) => {
            ui.colored_label(
                SUCCESS_COLOR,
                format!("Wrote {} and {}; it is open now, so further edits chain.", copy.display(), manifest.display()),
            );
        }
        Some(WriteOutcome::Failed(error)) => error_label(ui, format!("Not written: {error}")),
        None => {}
    }
    save
}
