use std::path::{Path, PathBuf};

use eframe::egui;

use crate::app::UddConvApp;
use crate::logic::settings::enhanced_client_dir;
use crate::models::{AssetPackProgressState, AssetPackTask, QuickConvertMode};
use crate::ui::{layout::page_heading, sources::pick_folder};

impl UddConvApp {
    pub(crate) fn ui_quick_convert(&mut self, ui: &mut egui::Ui) {
        ui.label(page_heading("Quick Convert"));
        ui.label("Build Dynamapper's UDDP packages with the default conversion settings.");
        ui.label("BC7 or BC1 where supported, with Zstd or adaptive LZ4. No upscaling or filters.");
        ui.add_space(ui.spacing().item_spacing.y);

        let is_busy = self.is_busy();
        ui.add_enabled_ui(!is_busy, |ui| {
            egui::Grid::new("quick_convert_folders").show(ui, |ui| {
                if let Some(path) = folder_row(ui, "Classic Client", self.settings.cc_dir.as_deref()) {
                    if self.settings.cc_dir.as_ref() != Some(&path) {
                        self.settings.classic_client_exe = None;
                        self.settings.cc_dir = Some(path);
                    }
                }
                if let Some(path) = folder_row(ui, "Enhanced Client", self.settings.ec_dir.as_deref()) {
                    self.settings.ec_dir = Some(enhanced_client_dir(path));
                }
                if let Some(path) = folder_row(ui, "Output UDDP folder", Some(&self.settings.output_uddp_dir)) {
                    if self.settings.link_uddp_dirs {
                        self.settings.input_uddp_dir = path.clone();
                    }
                    self.settings.output_uddp_dir = path;
                }
            });

            let classic_ready = self.settings.cc_dir.as_deref().is_some_and(Path::is_dir);
            let enhanced_ready = self.settings.ec_dir.as_deref().is_some_and(Path::is_dir);
            ui.add_space(ui.spacing().item_spacing.y);
            ui.horizontal_wrapped(|ui| {
                for (mode, label) in [
                    (QuickConvertMode::ClassicTextures, "Convert Classic textures"),
                    (QuickConvertMode::EnhancedTextures, "Convert Enhanced textures"),
                    (QuickConvertMode::Metadata, "Create metadata"),
                    (QuickConvertMode::Everything, "Create everything"),
                ] {
                    let button = ui.add_enabled(mode.is_available(classic_ready, enhanced_ready), egui::Button::new(label));
                    if button.clicked() {
                        self.start_quick_convert(mode);
                    }
                }
            });
            if enhanced_ready && !classic_ready {
                ui.label("Add a Classic Client folder for maps, statics, tile metadata, and Enhanced auxiliary textures.");
            }
        });

        let progress = self.asset_pack_progress(AssetPackTask::QuickConvert);
        if progress.state != AssetPackProgressState::Idle {
            ui.add_space(ui.spacing().item_spacing.y);
            ui.horizontal(|ui| {
                if progress.state == AssetPackProgressState::Running && ui.button("Stop").clicked() {
                    self.request_cancel_conversion();
                }
                ui.add(egui::ProgressBar::new(progress.fraction).text(progress.text));
            });
        }

        if let Some(mode) = self.quick_convert_overwrite {
            let mut replace = false;
            let mut cancel = false;
            let modal = egui::Modal::new(egui::Id::new("quick_convert_overwrite")).show(ui.ctx(), |ui| {
                ui.heading("Replace existing packages?");
                ui.label(format!("This run will replace existing outputs in {}.", self.settings.output_uddp_dir.display()));
                if mode == QuickConvertMode::Metadata
                    && self.settings.cc_dir.as_deref().is_some_and(Path::is_dir)
                    && self.settings.ec_dir.as_deref().is_none_or(|path| !path.is_dir())
                    && self.get_output_path("tilemeta.uddp").exists()
                {
                    ui.label("Add an Enhanced Client folder to include Enhanced data in the rebuilt tile metadata.");
                }
                ui.horizontal(|ui| {
                    cancel = ui.button("Cancel").clicked();
                    replace = ui.button("Replace and convert").clicked();
                });
            });
            if replace || cancel || modal.should_close() {
                self.quick_convert_overwrite = None;
            }
            if replace {
                self.quick_convert(mode);
            }
        }
    }

    fn start_quick_convert(&mut self, mode: QuickConvertMode) {
        if self.quick_convert_outputs(mode).iter().any(|path| path.symlink_metadata().is_ok()) {
            self.quick_convert_overwrite = Some(mode);
        } else {
            self.quick_convert(mode);
        }
    }
}

fn folder_row(ui: &mut egui::Ui, label: &str, current: Option<&Path>) -> Option<PathBuf> {
    ui.label(label);
    let path = ui.button("Choose folder…").clicked().then(|| pick_folder(label, current)).flatten();
    if let Some(path) = current {
        ui.label(path.to_string_lossy());
    } else {
        ui.weak("Not set");
    }
    ui.end_row();
    path
}
