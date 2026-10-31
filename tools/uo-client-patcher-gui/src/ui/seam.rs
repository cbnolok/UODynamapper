//! Section 3: the Enhanced Client split of facets 0 and 1, the Classic wrap seam, or Kingdom Reborn's note.

use eframe::egui;
use uocf::client_exe::facet_dimensions::{ClientFamily, DISABLED_SPLIT_X, STOCK_SPLIT_X};

use super::{error_label, muted_label, warning_label, ITEM_SPACING};
use crate::app::Session;
use crate::logic::edits::SeamChoice;
use crate::logic::opened::{seam_status, seam_word, split_x_label};
use crate::logic::preview::seam_warnings;

const X_EDIT_WIDTH: f32 = 64.0;

pub fn ui_seam(ui: &mut egui::Ui, session: &mut Session) {
    let family = session.edits.family;
    ui.label(seam_status(&session.opened.inspection));
    match family {
        ClientFamily::KingdomReborn => {
            muted_label(
                ui,
                "Kingdom Reborn has no Felucca/Trammel split: nothing to move. Resizing facet 0 or 1 warns in its row.",
            );
            return;
        }
        ClientFamily::Enhanced => muted_label(
            ui,
            format!(
                "Facets 0 and 1 (Felucca, Trammel) do not build a chunk across x = split. A new layout moves it, or \
                 disables it ({DISABLED_SPLIT_X:#x}, above any width)."
            ),
        ),
        ClientFamily::Classic => muted_label(
            ui,
            format!(
                "The radar and the walk test stop wrapping at x = seam (stock {STOCK_SPLIT_X}); it should equal the wrap \
                 width of Felucca and Trammel."
            ),
        ),
    }
    let Some(current) = session.edits.seam.current else {
        warning_label(ui, format!("No {} located in this build: it is left as it is.", seam_word(family)));
        return;
    };
    ui.add_space(ITEM_SPACING);
    let felucca_wrap = session.edits.felucca_wrap_width();
    let seam = &mut session.edits.seam;
    ui.horizontal(|ui| {
        ui.radio_value(&mut seam.choice, SeamChoice::Keep, format!("Keep x = {current} ({})", split_x_label(current)));
        ui.radio_value(&mut seam.choice, SeamChoice::Move, "Move to x =");
        let edit = ui.add(egui::TextEdit::singleline(&mut seam.x).desired_width(X_EDIT_WIDTH));
        if edit.changed() {
            seam.choice = SeamChoice::Move;
        }
        ui.radio_value(&mut seam.choice, SeamChoice::Disable, format!("Disable ({DISABLED_SPLIT_X:#x})"));
        if let Some(wrap) = felucca_wrap
            && ui.button(format!("Seam at Felucca's wrap ({wrap})")).clicked()
        {
            seam.choice = SeamChoice::Move;
            seam.x = wrap.to_string();
        }
    });
    let (_, validation) = session.edits.check_seam();
    for error in &validation.errors {
        error_label(ui, error.as_str());
    }
    let warnings = session.preview.get(&session.opened, &session.edits).warnings().to_vec();
    for warning in seam_warnings(&warnings) {
        warning_label(ui, warning.as_str());
    }
}
