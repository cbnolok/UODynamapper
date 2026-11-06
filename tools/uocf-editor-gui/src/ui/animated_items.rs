//! What the two views of animated items share. `AnimationDefinition.uop` (Enhanced Client, Kingdom Reborn) and
//! `animdata.mul` (Classic) both turn an item id into a list of tiles with a timing: one tab holds both, with a switch
//! between them and one choice of art source and hue.

use eframe::egui;

use crate::app::{ArtSource, EcHueingMode, UocfEditorApp, ViewMode};
use crate::ui::{
    animation_definitions, animdata, art_viewer, enhanced_files_availability, folder_files_availability, gumps,
    source_choice, ClientSource, SourceAvailability, CLASSIC_FOLDER,
};

const ANIMDATA_FILE: &str = "animdata.mul";
const ANIMATION_DEFINITION_FILE: &str = "AnimationDefinition.uop";

/// The switch between the two files, on top of both views: animdata.mul is MUL, AnimationDefinition.uop KR UOP or EC
/// UOP by its client. The Classic client keeps no animated items in a UOP package: no CC UOP.
pub(super) fn source_switch(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    let cc_path = app.settings.cc_path.clone();
    let has_animdata = app.client_data.as_ref().is_some_and(|client| client.animdata.is_some());
    let client_package = animation_definitions::client_package(app);
    // A package opened from elsewhere counts as its client's.
    let opened = animation_definitions::opened_package(app);
    let opened_source = opened.as_ref().map(|(_, kingdom_reborn)| kr_or_ec(*kingdom_reborn));
    let selected = match app.view_mode {
        ViewMode::AnimData => Some(ClientSource::Mul),
        _ => opened_source.or(app.shown_enhanced).or(Some(ClientSource::EcUop)),
    };
    egui::Panel::top("animated_items_source").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.strong("Animated items");
            ui.separator();
            let clicked = source_choice(ui, selected, |source| match source {
                ClientSource::Mul => folder_files_availability(cc_path.as_deref(), CLASSIC_FOLDER, ANIMDATA_FILE, has_animdata),
                ClientSource::CcUop => SourceAvailability::NotApplicable,
                ClientSource::KrUop | ClientSource::EcUop => match &opened {
                    Some((path, _)) if opened_source == Some(source) && client_package.as_ref() != Some(path) => {
                        SourceAvailability::Found(format!("{ANIMATION_DEFINITION_FILE} opened from {}", path.display()))
                    }
                    _ => enhanced_files_availability(
                        app,
                        source,
                        ANIMATION_DEFINITION_FILE,
                        &[ANIMATION_DEFINITION_FILE],
                        client_package.is_some(),
                    ),
                },
            });
            match clicked {
                Some(ClientSource::Mul) => app.view_mode = ViewMode::AnimData,
                Some(source) => {
                    app.view_mode = ViewMode::AnimationDefinitions;
                    app.show_enhanced_client(source);
                }
                None => {}
            }
        });
    });
}

fn kr_or_ec(kingdom_reborn: bool) -> ClientSource {
    if kingdom_reborn { ClientSource::KrUop } else { ClientSource::EcUop }
}

/// Where the frames' art comes from.
pub(super) fn art_source_controls(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Art source:");
        let current = app.selected_animdata_art_source;
        if let Some(source) = art_viewer::art_source_choice(app, ui, current) {
            app.selected_animdata_art_source = source;
        }
    });
}

/// The hue of the frames, and which table it comes from ([`UocfEditorApp::get_animdata_art_texture_from_source`]).
pub(super) fn hue_controls(app: &mut UocfEditorApp, ui: &mut egui::Ui) {
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
    app: &mut UocfEditorApp,
    ctx: &egui::Context,
    tile_id: u32,
    frame_index: usize,
    frame_count: usize,
) -> Option<egui::TextureHandle> {
    let source = app.selected_animdata_art_source;
    let art_id = animdata::animdata_preview_art_id(tile_id, source);
    app.get_animdata_art_texture_from_source(ctx, art_id, source, frame_index, frame_count)
}
