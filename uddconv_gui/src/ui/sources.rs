use eframe::egui;
use crate::app::UddConvApp;

impl UddConvApp {
    pub fn ui_sources(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.heading("Source Directories");
                ui.label("Configure where your Ultima Online client files are located.");
                ui.add_space(10.0);

                egui::Grid::new("source_grid")
                    .num_columns(3)
                    .spacing([10.0, 10.0])
                    .show(ui, |ui| {
                        ui.label("Classic Client (CC):");
                        if ui.button("Select...").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                self.settings.cc_dir = Some(path);
                            }
                        }
                        if let Some(path) = &self.settings.cc_dir {
                            ui.label(path.to_string_lossy());
                        } else {
                            ui.colored_label(
                                egui::Color32::LIGHT_RED,
                                "Not selected (required for CC Art/Map)",
                            );
                        }
                        ui.end_row();

                        ui.label("Enhanced Client (EC):");
                        if ui.button("Select...").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                self.settings.ec_dir = Some(path);
                            }
                        }
                        if let Some(path) = &self.settings.ec_dir {
                            ui.label(path.to_string_lossy());
                        } else {
                            ui.colored_label(
                                egui::Color32::LIGHT_RED,
                                "Not selected (required for EC Art/Land)",
                            );
                        }
                        ui.end_row();

                        ui.label("Input UDDP Folder:");
                        ui.horizontal(|ui| {
                            if ui.button("Select...").clicked() {
                                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                    self.settings.input_uddp_dir = path.clone();
                                    if self.settings.link_uddp_dirs {
                                        self.settings.output_uddp_dir = path;
                                    }
                                }
                            }
                            ui.checkbox(&mut self.settings.link_uddp_dirs, "Link to Output");
                        });
                        ui.label(self.settings.input_uddp_dir.to_string_lossy());
                        ui.end_row();

                        ui.label("Output UDDP Folder:");
                        ui.add_enabled_ui(!self.settings.link_uddp_dirs, |ui| {
                            if ui.button("Select...").clicked() {
                                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                    self.settings.output_uddp_dir = path;
                                }
                            }
                        });
                        ui.label(self.settings.output_uddp_dir.to_string_lossy());
                        ui.end_row();
                    });
            });

            ui.add_space(15.0);
            ui.label("Atlas size and gutter are automatically managed for optimal compatibility.");
        });
    }
}
