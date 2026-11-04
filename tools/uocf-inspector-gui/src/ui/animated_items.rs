//! What the two views of animated items share. `AnimationDefinition.uop` (Enhanced Client, Kingdom Reborn) and
//! `animdata.mul` (Classic) both turn an item id into a list of tiles with a timing: one tab holds both, with a switch
//! between them and one choice of art source and hue.

use eframe::egui;

use crate::app::{ArtSource, EcHueingMode, UopInspectorApp, ViewMode};
use crate::ui::{animdata, gumps};

/// The switch between the two files, on top of both views.
pub(super) fn source_switch(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    egui::Panel::top("animated_items_source").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.strong("Animated items");
            ui.separator();
            ui.selectable_value(&mut app.view_mode, ViewMode::AnimationDefinitions, "AnimationDefinition.uop (EC, KR)");
            ui.selectable_value(&mut app.view_mode, ViewMode::AnimData, "animdata.mul (Classic)");
        });
    });
}

/// Where the frames' art comes from.
pub(super) fn art_source_controls(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Art source:");
        ui.radio_value(&mut app.selected_animdata_art_source, ArtSource::Mul, "MUL");
        ui.radio_value(&mut app.selected_animdata_art_source, ArtSource::CcUop, "CC UOP");
        ui.radio_value(&mut app.selected_animdata_art_source, ArtSource::EcUopLegacy, "EC Legacy UOP");
        ui.radio_value(&mut app.selected_animdata_art_source, ArtSource::EcUopKr, "KR/New UOP");
    });
}

/// The hue of the frames, and which table it comes from ([`UopInspectorApp::get_animdata_art_texture_from_source`]).
pub(super) fn hue_controls(app: &mut UopInspectorApp, ui: &mut egui::Ui) {
    let classic_hues = app.client_data.as_ref().is_some_and(|client| client.hues.is_some());
    let enhanced_hues = app.has_loaded_uop(uocf::enhanced::hues::HUES_UOP_NAME);
    let ec_source = matches!(app.selected_animdata_art_source, ArtSource::EcUopLegacy | ArtSource::EcUopKr);
    let uses_enhanced_hues = enhanced_hues && (ec_source || !classic_hues);
    ui.horizontal_wrapped(|ui| {
        ui.label("Hue:");
        ui.add(egui::DragValue::new(&mut app.selected_hue_id)).on_hover_text("0 shows the art as it is stored.");
        if uses_enhanced_hues {
            ui.weak("from Hues.uop, mapped as");
            ui.selectable_value(&mut app.selected_ec_hueing_mode, EcHueingMode::Cc, "CC");
            ui.selectable_value(&mut app.selected_ec_hueing_mode, EcHueingMode::Ec, "EC");
        } else if classic_hues && !ec_source {
            ui.weak("from hues.mul");
        } else {
            ui.weak("no hue table for this art source: load Hues.uop");
        }
    });
    if classic_hues {
        egui::CollapsingHeader::new("Hue swatches").id_salt("animated_items_hue_swatches").show(ui, |ui| gumps::ui_hue_picker(app, ui));
    }
}

/// A frame's art, in the chosen source and hue. `None` when that source has no art for the tile.
pub(super) fn frame_texture(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    tile_id: u32,
    frame_index: usize,
    frame_count: usize,
) -> Option<egui::TextureHandle> {
    let source = app.selected_animdata_art_source;
    let art_id = animdata::animdata_preview_art_id(tile_id, source);
    app.get_animdata_art_texture_from_source(ctx, art_id, source, frame_index, frame_count)
}
