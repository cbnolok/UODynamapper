use eframe::egui;
use crate::app::UddConvApp;

impl UddConvApp {
    pub fn ui_tools(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.heading("Package Tools");
            ui.label("Inspect, extract and compare UDDP packages.");
            ui.add_space(15.0);

            let is_busy = self.is_busy();

            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.heading("Inspect & Extract");
                ui.add_space(5.0);

                // File selector for Tool 1
                ui.horizontal(|ui| {
                    if ui.button("📁 Select Package...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("UDDP Packages", &["uddp", "uddpi"])
                            .pick_file()
                        {
                            self.tool_file_1 = Some(path);
                        }
                    }

                    let Some(path) = &self.tool_file_1 else {
                        return;
                    };

                    ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                    if ui.button("❌").clicked() {
                        self.tool_file_1 = None;
                    }
                });

                ui.add_space(5.0);
                // Action buttons (requires a selected file)
                ui.add_enabled_ui(!is_busy && self.tool_file_1.is_some(), |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("ℹ️ Get Info").clicked() {
                            self.tool_info();
                        }
                        if ui.button("📦 Extract Contents").clicked() {
                            self.tool_extract();
                        }
                        if ui.button("🧾 Export CSV Metadata").clicked() {
                            self.tool_export_csv();
                        }
                    });
                });
            });

            ui.add_space(15.0);

            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.heading("Virtual Path Hash");
                ui.add_space(5.0);

                ui.horizontal(|ui| {
                    ui.label("Path:");
                    ui.text_edit_singleline(&mut self.tool_hash_value);
                    if ui
                        .add_enabled(!self.tool_hash_value.trim().is_empty(), egui::Button::new("Compute Hash"))
                        .clicked()
                    {
                        self.tool_hash_path();
                    }
                });
            });

            ui.add_space(15.0);

            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.heading("Compare Packages (Diff)");
                ui.add_space(5.0);

                // Grid for selecting two files for comparison
                egui::Grid::new("diff_grid").num_columns(2).show(ui, |ui| {
                    ui.label("Package A:");
                    ui.horizontal(|ui| {
                        if ui.button("Select...").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_file() {
                                self.tool_file_1 = Some(path);
                            }
                        }
                        let Some(path) = &self.tool_file_1 else {
                            return;
                        };
                        ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                    });
                    ui.end_row();

                    ui.label("Package B:");
                    ui.horizontal(|ui| {
                        if ui.button("Select...").clicked() {
                            if let Some(path) = rfd::FileDialog::new().pick_file() {
                                self.tool_file_2 = Some(path);
                            }
                        }
                        let Some(path) = &self.tool_file_2 else {
                            return;
                        };
                        ui.label(path.file_name().unwrap_or_default().to_string_lossy());
                    });
                    ui.end_row();
                });

                ui.add_space(10.0);
                // Diff action (requires both files selected)
                ui.add_enabled_ui(!is_busy && self.tool_file_1.is_some() && self.tool_file_2.is_some(), |ui| {
                    if ui.button("🔍 Run Diff").clicked() {
                        self.tool_diff();
                    }
                });
            });
        });
    }
}
