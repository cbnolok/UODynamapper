use eframe::egui;
use crate::app::UopInspectorApp;

pub mod animations;
pub mod art_viewer;
pub mod uop_browser;
pub mod multis;
pub mod hues;
pub mod terrain_definition;

pub fn draw_ui(app: &mut UopInspectorApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open UOP...").clicked() {
                    app.open_uop();
                    ui.close_menu();
                }
                if ui.button("Search Paths...").clicked() {
                    app.show_search_paths = true;
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Exit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            
            ui.separator();
            
            ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Home, "Home");
            ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::UopExplorer, "UOP Explorer");
            if app.client_data.is_some() {
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::TexArtCc, "CC Art");
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::CcTileData, "CC TileData");
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Multis, "Multis");
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Hues, "Hues");
            }
            ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::Animations, "Animations");
            if app.terrain_def_package.is_some() {
                ui.selectable_value(&mut app.view_mode, crate::app::ViewMode::TerrainDefinition, "Terrain Def");
            }
        });
    });

    egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.label(&app.status_message);
        });
    });

    let mut show_search_paths = app.show_search_paths;
    if show_search_paths {
        egui::Window::new("Search Paths")
            .open(&mut show_search_paths)
            .show(ctx, |ui| {
                egui::Grid::new("paths_grid").show(ui, |ui| {
                    ui.label("CC Path:");
                    ui.horizontal(|ui| {
                        if let Some(path) = &app.settings.cc_path {
                            ui.label(path.display().to_string());
                        } else {
                            ui.label("None");
                        }
                        if ui.button("Select...").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                app.settings.cc_path = Some(path);
                                app.trigger_reload();
                            }
                        }
                    });
                    ui.end_row();

                    ui.label("EC Path:");
                    ui.horizontal(|ui| {
                        if let Some(path) = &app.settings.ec_path {
                            ui.label(path.display().to_string());
                        } else {
                            ui.label("None");
                        }
                        if ui.button("Select...").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                app.settings.ec_path = Some(path);
                                app.trigger_reload();
                            }
                        }
                    });
                    ui.end_row();

                    ui.label("Dictionary (.bin):");
                    ui.horizontal(|ui| {
                        if let Some(path) = &app.settings.dict_path {
                            ui.label(path.display().to_string());
                        } else {
                            ui.label("None");
                        }
                        if ui.button("Select...").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Binary Dictionary", &["bin"])
                                .pick_file()
                            {
                                app.settings.dict_path = Some(path);
                                app.trigger_reload();
                            }
                        }
                    });
                    ui.end_row();
                });
            });
        app.show_search_paths = show_search_paths;
    }

    match app.view_mode {
        crate::app::ViewMode::Home => {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("Logs");
                ui.separator();
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for log in &app.logs {
                            ui.label(log);
                        }
                    });
            });
        }
        crate::app::ViewMode::UopExplorer => {
            uop_browser::ui_uop_browser(app, ctx);
        }
        crate::app::ViewMode::TexArtCc | crate::app::ViewMode::CcTileData => {
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
        crate::app::ViewMode::TerrainDefinition => {
            terrain_definition::ui_terrain_definition(app, ctx);
        }
    }
}
