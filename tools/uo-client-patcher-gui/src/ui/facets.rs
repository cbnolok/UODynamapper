//! Section 2: one row per facet, edited and validated live.

use eframe::egui;
use uocf::client_exe::facet_dimensions::{ClientFamily, FACET_SIDE_ALIGNMENT};
use uocf::enhanced::facet_sector::{FacetGeometry, SECTOR_SIDE};

use super::{cell, error_label, muted_label, warning_label, ERROR_COLOR, ITEM_SPACING, SUCCESS_COLOR, WARNING_COLOR};
use crate::app::Session;
use crate::logic::edits::{size_units, EditState};
use crate::logic::opened::classic_files;

const INDEX_WIDTH: f32 = 24.0;
const NAME_WIDTH: f32 = 100.0;
const STOCK_WIDTH: f32 = 90.0;
const CURRENT_WIDTH: f32 = 90.0;
/// The current size and wrap (Classic).
const CURRENT_WITH_WRAP_WIDTH: f32 = 170.0;
const EDIT_WIDTH: f32 = 56.0;
const RULES_WIDTH: f32 = 50.0;
const UNITS_WIDTH: f32 = 150.0;
const CLASSIC_UNITS_WIDTH: f32 = 100.0;
const BUTTON_WIDTH: f32 = 52.0;
const STATUS_WIDTH: f32 = 24.0;
/// Messages and file ids start under the name column.
const DETAIL_INDENT: f32 = INDEX_WIDTH;

fn size_text(size: FacetGeometry) -> String {
    format!("{} x {}", size.width, size.height)
}

struct Columns {
    classic: bool,
}

impl Columns {
    fn current(&self) -> f32 {
        if self.classic { CURRENT_WITH_WRAP_WIDTH } else { CURRENT_WIDTH }
    }

    fn units(&self) -> f32 {
        if self.classic { CLASSIC_UNITS_WIDTH } else { UNITS_WIDTH }
    }
}

fn header(ui: &mut egui::Ui, columns: &Columns, family: ClientFamily) {
    let strong = |text: &str| egui::RichText::new(text).strong();
    ui.horizontal(|ui| {
        cell(ui, INDEX_WIDTH, |ui| ui.label(strong("#")));
        cell(ui, NAME_WIDTH, |ui| ui.label(strong("Facet")));
        cell(ui, STOCK_WIDTH, |ui| ui.label(strong("Stock")));
        cell(ui, columns.current(), |ui| ui.label(strong("In this file")));
        cell(ui, EDIT_WIDTH, |ui| ui.label(strong("Width")));
        cell(ui, EDIT_WIDTH, |ui| ui.label(strong("Height")));
        if columns.classic {
            cell(ui, EDIT_WIDTH, |ui| ui.label(strong("Wrap W")));
            cell(ui, EDIT_WIDTH, |ui| ui.label(strong("Wrap H")));
            cell(ui, RULES_WIDTH, |ui| ui.label(strong("Rules"))).on_hover_text("felucca_rules: 1 for Felucca");
        }
        let units = if family == ClientFamily::Classic { "Map blocks" } else { "Sectors" };
        cell(ui, columns.units(), |ui| ui.label(strong(units)));
    });
}

pub fn ui_facets(ui: &mut egui::Ui, session: &mut Session, hide_file_ids: &mut bool) {
    let inspection = &session.opened.inspection;
    let edits = &mut session.edits;
    let columns = Columns { classic: edits.is_classic() };
    ui.horizontal(|ui| {
        muted_label(
            ui,
            format!(
                "Sizes in squares, multiples of {FACET_SIDE_ALIGNMENT} ({SECTOR_SIDE} avoids padded sectors). Edits \
                 are checked as you type."
            ),
        );
        if columns.classic {
            let mut show = !*hide_file_ids;
            if ui.checkbox(&mut show, "Show file ids").changed() {
                *hide_file_ids = !show;
            }
        }
    });
    if columns.classic {
        muted_label(
            ui,
            "A wrap side equal to the old size follows a new size until you type a wrap. Both table copies and the \
             startup globals of facet 0 are written.",
        );
    }
    ui.add_space(ITEM_SPACING);
    egui::ScrollArea::horizontal().id_salt("facet_rows").show(ui, |ui| {
        ui.vertical(|ui| {
            header(ui, &columns, edits.family);
            ui.separator();
            for index in 0..edits.rows.len() {
                let files = classic_files(inspection, index);
                facet_row(ui, edits, &columns, index, files.as_ref().map(|(_, rules)| *rules));
                if !*hide_file_ids
                    && let Some((files, _)) = files
                {
                    let copies = files.len();
                    for (copy, line) in files.into_iter().enumerate() {
                        let prefix = if copies > 1 { format!("copy {}: ", copy + 1) } else { String::new() };
                        ui.horizontal(|ui| {
                            ui.add_space(DETAIL_INDENT);
                            ui.label(egui::RichText::new(format!("{prefix}{line}")).small().weak());
                        });
                    }
                }
            }
        });
    });
    edits.sync_followed_wraps();
}

fn size_edit(ui: &mut egui::Ui, text: &mut String, invalid: bool) -> egui::Response {
    let color = invalid.then_some(ERROR_COLOR);
    ui.add(egui::TextEdit::singleline(text).desired_width(EDIT_WIDTH).text_color_opt(color))
}

/// One row and its messages; `felucca_rules` is the Classic record's flag.
fn facet_row(ui: &mut egui::Ui, edits: &mut EditState, columns: &Columns, index: usize, felucca_rules: Option<u32>) {
    let family = edits.family;
    let check = edits.check_row(index);
    let can_reset_to_stock = edits.can_reset_to_stock(index);
    let invalid = !check.validation.is_ok();
    let mut revert = false;
    let mut to_stock = false;
    ui.horizontal(|ui| {
        let row = &mut edits.rows[index];
        cell(ui, INDEX_WIDTH, |ui| ui.label(row.facet.to_string()));
        cell(ui, NAME_WIDTH, |ui| ui.label(&row.name));
        let stock = row.stock.map_or_else(|| "—".to_owned(), size_text);
        let stock = cell(ui, STOCK_WIDTH, |ui| ui.label(stock));
        if columns.classic && row.stock.is_some() && row.stock != Some(row.current) {
            stock.on_hover_text("The Mondain's Legacy and later sizes; older builds shipped other Felucca and Trammel widths");
        }
        let current = match row.current_wrap {
            Some(wrap) => format!("{} (wrap {})", size_text(row.current), size_text(wrap)),
            None => size_text(row.current),
        };
        let differs = row.stock.is_some_and(|stock| stock != row.current);
        cell(ui, columns.current(), |ui| {
            let text = egui::RichText::new(current);
            ui.label(if differs { text.color(WARNING_COLOR) } else { text })
        });
        size_edit(ui, &mut row.width, invalid);
        size_edit(ui, &mut row.height, invalid);
        if let Some(current_wrap) = row.current_wrap {
            let hint = format!("In this file: {}", size_text(current_wrap));
            let wrap_width = size_edit(ui, &mut row.wrap_width, invalid).on_hover_text(&hint);
            let wrap_height = size_edit(ui, &mut row.wrap_height, invalid).on_hover_text(&hint);
            if wrap_width.changed() || wrap_height.changed() {
                row.wrap_touched = true;
            }
        }
        if columns.classic {
            let rules = felucca_rules.map_or("—", |rules| if rules != 0 { "yes" } else { "no" });
            cell(ui, RULES_WIDTH, |ui| ui.label(rules));
        }
        let units = check.size.map_or_else(|| "—".to_owned(), |size| size_units(family, size));
        cell(ui, columns.units(), |ui| ui.label(units));
        revert = ui
            .add_enabled(row.is_edited(), egui::Button::new("Revert").min_size(egui::vec2(BUTTON_WIDTH, 0.0)))
            .on_hover_text("Back to the values in this file")
            .clicked();
        to_stock = ui
            .add_enabled(can_reset_to_stock, egui::Button::new("Stock").min_size(egui::vec2(BUTTON_WIDTH, 0.0)))
            .on_hover_text("The shipped size (and wrap)")
            .clicked();
        let (mark, color, tip) = if invalid {
            ("✖", ERROR_COLOR, "errors")
        } else if !check.validation.warnings.is_empty() {
            ("⚠", WARNING_COLOR, "warnings")
        } else if row.is_edited() {
            ("✔", SUCCESS_COLOR, "valid edit")
        } else {
            ("", SUCCESS_COLOR, "")
        };
        cell(ui, STATUS_WIDTH, |ui| ui.colored_label(color, mark)).on_hover_text(tip);
    });
    if revert {
        edits.reset_to_current(index);
    }
    if to_stock {
        edits.reset_to_stock(index);
    }
    for error in &check.validation.errors {
        ui.horizontal(|ui| {
            ui.add_space(DETAIL_INDENT);
            error_label(ui, error.as_str());
        });
    }
    for warning in &check.validation.warnings {
        ui.horizontal(|ui| {
            ui.add_space(DETAIL_INDENT);
            warning_label(ui, warning.as_str());
        });
    }
}
