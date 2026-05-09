use eframe::egui;
use crate::app::UopInspectorApp;

pub mod animations;
pub mod art_viewer;
pub mod uop_browser;
pub mod multis;
pub mod hues;

pub fn draw_ui(app: &mut UopInspectorApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open UOP...").clicked() {
                    app.open_uop();
                    ui.close_menu();
                }
                if ui.button("Select Client Dir...").clicked() {
                    app.open_client_dir();
                    ui.close_menu();
                }
                if ui.button("Open Client Dir (CC MUL)...").clicked() {
                    app.open_client_dir();
                    ui.close_menu();
                }
                if ui.button("Open LegacyTexture.uop...").clicked() {
                    app.open_legacy_texture_uop();
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Load Dictionary (.dic)...").clicked() {
                    app.open_dictionary();
                    ui.close_menu();
                }
                if ui.button("Load Binary Dictionary (.bin)...").clicked() {
                    app.open_bin_dictionary();
                    ui.close_menu();
                }
                if ui.button("Load UO String Dictionary (string_dictionary.uop)...").clicked() {
                    app.open_uo_string_dictionary();
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            
            ui.separator();
            
            ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::UopExplorer, "UOP Explorer");
            if app.client_data.is_some() {
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::CcArt, "CC Art");
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::CcTileData, "CC TileData");
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Multis, "Multis");
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Hues, "Hues");
            }
            ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Animations, "Animations");
        });
    });

    egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(&app.status_message);
        });
    });

    match app.view_mode {
        crate::app::ViewMode::UopExplorer => {
            uop_browser::ui_uop_browser(app, ctx);
        }
        crate::app::ViewMode::CcArt | crate::app::ViewMode::CcTileData => {
            art_viewer::ui_art_viewer(app, ctx);
        }
        crate::app::ViewMode::Animations => {
            animations::ui_animations(app, ctx);
        }
        crate::app::ViewMode::Multis => {
            multis::ui_multis(app, ctx);
        }
        crate::app::ViewMode::Hues => {
            hues::ui_hues(app, ctx);
        }
    }
}
