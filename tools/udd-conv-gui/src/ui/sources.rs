use eframe::egui;
use crate::app::UddConvApp;
use crate::ui::layout::page_heading;
use std::path::{Path, PathBuf};

impl UddConvApp {
    pub fn ui_sources(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.label(page_heading("Source Directories"));
            ui.label("Configure where your Ultima Online client files are located.");
            ui.add_space(10.0);

            ui.group(|ui| {
                ui.set_width(ui.available_width());
                egui::Grid::new("source_grid")
                    .num_columns(3)
                    .spacing([10.0, 10.0])
                    .show(ui, |ui| {
                        ui.label("Classic Client (CC):");
                        if ui.button("Select Folder...").clicked() {
                            if let Some(path) = pick_folder(
                                "Select Classic Client (CC) Directory",
                                self.settings.cc_dir.as_deref(),
                            ) {
                                if self.settings.cc_dir.as_ref() != Some(&path) {
                                    self.settings.classic_client_exe = None;
                                }
                                self.settings.cc_dir = Some(path);
                                self.persist_settings();
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
                        if ui.button("Select Folder...").clicked() {
                            if let Some(path) = pick_folder(
                                "Select Enhanced Client (EC) Directory",
                                self.settings.ec_dir.as_deref(),
                            ) {
                                self.settings.ec_dir = Some(path);
                                self.persist_settings();
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

                        ui.label("Kingdom Reborn Client (KR):");
                        if ui.button("Select Folder...").clicked() {
                            if let Some(path) = pick_folder(
                                "Select Kingdom Reborn Client (KR) Directory",
                                self.settings.kr_dir.as_deref(),
                            ) {
                                self.settings.kr_dir = Some(path);
                                self.persist_settings();
                            }
                        }
                        if let Some(path) = &self.settings.kr_dir {
                            ui.label(path.to_string_lossy());
                        } else {
                            ui.colored_label(
                                egui::Color32::YELLOW,
                                "Not selected (needed only for Kingdom Reborn Land)",
                            );
                        }
                        ui.end_row();

                        ui.horizontal(|ui| {
                            ui.label("Dynamapper Routing Files:");
                            ui.label(egui::RichText::new("(?)").color(egui::Color32::from_rgb(100, 200, 255)))
                                .on_hover_text(
                                    "Directory containing KDL metadata files shipped with UODynamapper — \
                                    not part of the UO client.\n\n\
                                    Used to locate EcMobileAnimations.kdl for EC mobile animation packing, and \
                                    EcTerrainOverrides.kdl and TerrainTranscode.kdl (the land routing table, \
                                    embedded in tex_land_ec.uddp) for EC land packing. If not set, the packer uses the \
                                    copies shipped with the tools (shared/cc_ec_convtables/ in a release, \
                                    dynamapper/assets/cc_ec_convtables/ in a source checkout), so this field is \
                                    only needed to pack with edited tables."
                                );
                        });
                        if ui.button("Select Folder...").clicked() {
                            if let Some(path) = pick_folder(
                                "Select Dynamapper Routing Files Directory",
                                self.settings.dynamapper_routing_dir.as_deref(),
                            ) {
                                self.settings.dynamapper_routing_dir = Some(path);
                                self.persist_settings();
                            }
                        }
                        if let Some(path) = &self.settings.dynamapper_routing_dir {
                            ui.label(path.to_string_lossy());
                        } else {
                            ui.colored_label(
                                egui::Color32::YELLOW,
                                "Not selected (uses the tables shipped with the tools)",
                            );
                        }
                        ui.end_row();

                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            ui.label(egui::RichText::new("↳ EC Mobile Animation KDL:").weak());
                        });
                        if ui.checkbox(
                            &mut self.settings.ec_mobile_anim_allow_missing_kdl,
                            "Allow conversion without EcMobileAnimations.kdl",
                        )
                        .on_hover_text("Writes empty EC mobile animation item/source-hint metadata when the KDL is unavailable.")
                        .changed()
                        {
                            self.persist_settings();
                        }
                        ui.label("");
                        ui.end_row();

                        ui.horizontal(|ui| {
                            ui.label("Input UDDP Folder:");
                            ui.label(egui::RichText::new("(?)").color(egui::Color32::from_rgb(100, 200, 255)))
                                .on_hover_text(
                                    "Folder where already-built .uddp packages are read from as \
                                    inputs to dependent conversions:\n\n\
                                    • Radar map (facet0X.*) — reads tilemeta.uddp\n\
                                    • EC Auxiliary Textures (tex_aux_ec.uddp) — reads tilemeta.uddp\n\n\
                                    Typically the same folder as the Output UDDP Folder. \
                                    Use 'Link to Output' to keep them in sync automatically."
                                );
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Select Folder...").clicked() {
                                if let Some(path) = pick_folder(
                                    "Select Input UDDP Folder",
                                    Some(&self.settings.input_uddp_dir),
                                ) {
                                    self.settings.input_uddp_dir = path.clone();
                                    if self.settings.link_uddp_dirs {
                                        self.settings.output_uddp_dir = path;
                                    }
                                    self.persist_settings();
                                }
                            }
                            if ui.checkbox(&mut self.settings.link_uddp_dirs, "Link to Output").changed() {
                                if self.settings.link_uddp_dirs {
                                    self.settings.output_uddp_dir = self.settings.input_uddp_dir.clone();
                                }
                                self.persist_settings();
                            }
                        });
                        ui.label(self.settings.input_uddp_dir.to_string_lossy());
                        ui.end_row();

                        ui.label("Output UDDP Folder:");
                        ui.add_enabled_ui(!self.settings.link_uddp_dirs, |ui| {
                            if ui.button("Select Folder...").clicked() {
                                if let Some(path) = pick_folder(
                                    "Select Output UDDP Folder",
                                    Some(&self.settings.output_uddp_dir),
                                ) {
                                    self.settings.output_uddp_dir = path;
                                    self.persist_settings();
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

fn pick_folder(title: &str, current: Option<&Path>) -> Option<PathBuf> {
    match pick_folder_with_kdialog(title, current) {
        KdialogFolderResult::Selected(path) => return Some(path),
        KdialogFolderResult::Cancelled => return None,
        KdialogFolderResult::Unavailable => {}
    }

    let mut dialog = uot_gui::file_dialog().set_title(title);
    if let Some(current) = current.filter(|path| path.is_dir()) {
        dialog = dialog.set_directory(current);
    }
    dialog.pick_folder()
}

enum KdialogFolderResult {
    Selected(PathBuf),
    Cancelled,
    Unavailable,
}

#[cfg(target_os = "linux")]
fn pick_folder_with_kdialog(title: &str, current: Option<&Path>) -> KdialogFolderResult {
    if !is_kde_session() {
        return KdialogFolderResult::Unavailable;
    }

    let mut command = std::process::Command::new("kdialog");
    command.arg("--getexistingdirectory");
    if let Some(current) = current.filter(|path| path.is_dir()) {
        command.arg(current);
    }
    command.args(["--title", title]);

    let output = match command.output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return KdialogFolderResult::Unavailable;
        }
        Err(error) => {
            log::debug!("Failed to open kdialog folder picker: {error}");
            return KdialogFolderResult::Unavailable;
        }
    };

    if !output.status.success() {
        return KdialogFolderResult::Cancelled;
    }

    let path = String::from_utf8_lossy(&output.stdout);
    let path = path.trim_end_matches(['\r', '\n']);
    if path.is_empty() {
        KdialogFolderResult::Cancelled
    } else {
        KdialogFolderResult::Selected(PathBuf::from(path))
    }
}

#[cfg(not(target_os = "linux"))]
fn pick_folder_with_kdialog(_title: &str, _current: Option<&Path>) -> KdialogFolderResult {
    KdialogFolderResult::Unavailable
}

#[cfg(target_os = "linux")]
fn is_kde_session() -> bool {
    ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP", "DESKTOP_SESSION"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .any(|value| {
            value
                .split(':')
                .any(|part| part.eq_ignore_ascii_case("KDE") || part.eq_ignore_ascii_case("PLASMA"))
        })
}
