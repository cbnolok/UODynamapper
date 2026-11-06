use eframe::egui;
use crate::app::UddConvApp;
use std::path::PathBuf;

/// The files the Validate Packages tool's file dialog offers: packages and the terrain metadata.
const VALIDATE_EXTENSIONS: [&str; 2] = ["uddp", "bin"];
const VALIDATE_HINT: &str = "Checks each package as udd-cli tool validate does: the container, every entry decoded, \
and the reader of its kind. A folder is searched for *.uddp and terrain_meta.bin.";
const VALIDATE_TOOLTIP: &str = "Validate every package listed, and those in the folders listed; the results go to the \
log.";

/// Adds the paths not listed yet, in order.
fn add_validate_paths(listed: &mut Vec<PathBuf>, paths: impl IntoIterator<Item = PathBuf>) {
    for path in paths {
        if !listed.contains(&path) {
            listed.push(path);
        }
    }
}

impl UddConvApp {
    pub fn ui_tools(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.heading("Package Tools");
            ui.label("Inspect, validate, extract and compare UDDP packages.");
            ui.add_space(15.0);

            let is_busy = self.is_busy();

            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.heading("Inspect & Extract");
                ui.add_space(5.0);

                // File selector for Tool 1
                ui.horizontal(|ui| {
                    if ui.button("📁 Select Package...").clicked() {
                        if let Some(path) = uot_gui::open_filtered_file_dialog(
                            None,
                            "UDDP Packages",
                            &["uddp", "uddpi"],
                        ) {
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

            self.ui_validate_packages(ui, is_busy);

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
                            if let Some(path) = uot_gui::file_dialog().pick_file() {
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
                            if let Some(path) = uot_gui::file_dialog().pick_file() {
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

    fn ui_validate_packages(&mut self, ui: &mut egui::Ui, is_busy: bool) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.heading("Validate Packages");
            ui.label(egui::RichText::new(VALIDATE_HINT).weak());
            ui.add_space(5.0);

            ui.horizontal(|ui| {
                if ui.button("📁 Add Packages...").clicked() {
                    if let Some(paths) = uot_gui::file_dialog()
                        .set_title("Select Packages to Validate")
                        .add_filter("Packages", &VALIDATE_EXTENSIONS)
                        .pick_files()
                    {
                        add_validate_paths(&mut self.tool_validate_paths, paths);
                    }
                }
                if ui.button("📁 Add Folder...").clicked() {
                    if let Some(dir) = uot_gui::pick_folder() {
                        add_validate_paths(&mut self.tool_validate_paths, [dir]);
                    }
                }
                if ui.add_enabled(!self.tool_validate_paths.is_empty(), egui::Button::new("Clear")).clicked() {
                    self.tool_validate_paths.clear();
                }
            });

            let mut removed = None;
            for (index, path) in self.tool_validate_paths.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.small_button("❌").on_hover_text("Remove from the list").clicked() {
                        removed = Some(index);
                    }
                    ui.label(path.display().to_string());
                    let kind = if path.is_dir() { "folder" } else { "package" };
                    ui.label(egui::RichText::new(kind).weak());
                });
            }
            if let Some(index) = removed {
                self.tool_validate_paths.remove(index);
            }

            ui.add_space(5.0);
            let can_validate = !is_busy && !self.tool_validate_paths.is_empty();
            let refusal = if is_busy { "Another task is running" } else { "Add packages or folders first" };
            if ui
                .add_enabled(can_validate, egui::Button::new("✔ Validate"))
                .on_hover_text(VALIDATE_TOOLTIP)
                .on_disabled_hover_text(refusal)
                .clicked()
            {
                self.tool_validate();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_paths_are_listed_once_in_the_order_added() {
        let mut listed = vec![PathBuf::from("/out/hues.uddp")];

        add_validate_paths(
            &mut listed,
            [PathBuf::from("/out"), PathBuf::from("/out/hues.uddp"), PathBuf::from("/out/terrain_meta.bin")],
        );

        assert_eq!(
            listed,
            [PathBuf::from("/out/hues.uddp"), PathBuf::from("/out"), PathBuf::from("/out/terrain_meta.bin")]
        );
    }
}
