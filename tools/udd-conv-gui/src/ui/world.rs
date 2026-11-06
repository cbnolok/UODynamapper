use crate::app::UddConvApp;
use crate::logic::dif_check::{exe_line, status_detail, status_line, DifCheckRequest, CLASSIC_CLIENT_EXE_NAME};
use eframe::egui;
use udd_conv::cc_radar::RadarFormat;
use uocf::client_exe::classic_difs::DifKind;
use uocf::map_edition::MapEdition;

const MAP_EDITION_HINT: &str = "Standard: mapN, staidxN, staticsN. Variant: the x files (mapNx, staidxNx, staticsNx) \
the server selects with the 0xA9 flag 0x8000, for maps 0, 1, 2 and 5 (the others read their standard files); a \
missing x file is an error, never a fallback. The dif options apply to whichever edition is selected.";
const DIF_CHECK_HINT: &str = "What the Classic client executable does with the map and static dif files, read from \
its code (information only: the dif options above stay your choice). A client applies them only once the server \
sends the patch counts (0xBF/0x18).";
const DIF_CHECK_INDENT: f32 = 16.0;

impl UddConvApp {
    pub fn ui_world(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.heading("World Data Packing");
            ui.label("Convert map and statics mul files into optimized block packages.");
            ui.add_space(15.0);

            let is_busy = self.is_busy();

            ui.add_enabled_ui(!is_busy, |ui| {
                ui.horizontal(|ui| {
                    ui.label("RadarMap texture compression:");
                    egui::ComboBox::from_id_salt("radar_fmt_shared")
                        .selected_text(format!("{:?}", self.settings.radar_format))
                        .width(100.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.settings.radar_format,
                                RadarFormat::Rgba8,
                                "RGBA8",
                            );
                            ui.selectable_value(
                                &mut self.settings.radar_format,
                                RadarFormat::Bc7,
                                "BC7",
                            );
                            ui.selectable_value(
                                &mut self.settings.radar_format,
                                RadarFormat::Bc7Ktx2,
                                "Supercompressed BC7+zstd",
                            );
                        });
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Classic patch files:");
                    ui.checkbox(&mut self.settings.include_verdata, "verdata.mul");
                    ui.checkbox(&mut self.settings.include_map_difs, "map difs");
                    ui.checkbox(&mut self.settings.include_static_difs, "static difs");
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Map edition:").on_hover_text(MAP_EDITION_HINT);
                    for edition in MapEdition::ALL {
                        ui.radio_value(&mut self.settings.map_edition, edition, edition.label())
                            .on_hover_text(MAP_EDITION_HINT);
                    }
                });
            });

            self.ui_dif_check(ui);

            ui.add_space(10.0);

            for map_id in 0..=5 {
                ui.group(|ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.set_min_width(80.0);
                        ui.label(egui::RichText::new(format!("Map {}", map_id)).strong());

                        ui.add_enabled_ui(!is_busy, |ui| {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 5.0;

                                if ui.button("Map").clicked() {
                                    self.convert_map(map_id);
                                }
                                if ui.button("Statics").clicked() {
                                    self.convert_statics(map_id);
                                }
                                if ui.button("RadarMap").clicked() {
                                    self.convert_radar(map_id);
                                }

                                let radar_path = self.get_output_path(&format!(
                                    "facet0{}.{}",
                                    map_id,
                                    self.settings.radar_format.extension()
                                ));
                                if radar_path.exists() {
                                    if ui.button("👁 View").clicked() {
                                        self.show_preview(&radar_path);
                                    }
                                }

                                ui.separator();

                                if ui
                                    .button(
                                        egui::RichText::new("ALL")
                                            .color(egui::Color32::from_rgb(100, 200, 255)),
                                    )
                                    .clicked()
                                {
                                    self.convert_all(map_id);
                                }
                            });
                        });
                    });
                });
                ui.add_space(5.0);
            }
        });
    }

    /// The Classic client's dif usage: the executable it was read from, one line per kind and the evidence.
    fn ui_dif_check(&mut self, ui: &mut egui::Ui) {
        let request = DifCheckRequest {
            cc_dir: self.settings.cc_dir.clone(),
            chosen_exe: self.settings.classic_client_exe.clone(),
        };
        let repaint_ctx = ui.ctx().clone();
        self.dif_check.update(request, move || repaint_ctx.request_repaint());

        let automatic_label = format!("Automatic ({CLASSIC_CLIENT_EXE_NAME})");
        let folder_exes = self.dif_check.report.as_ref().map(|report| report.folder_exes.clone()).unwrap_or_default();
        ui.horizontal_wrapped(|ui| {
            ui.label("Client dif usage:").on_hover_text(DIF_CHECK_HINT);
            let selected = match &self.settings.classic_client_exe {
                Some(path) => path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned()),
                None => automatic_label.clone(),
            };
            egui::ComboBox::from_id_salt("dif_check_exe")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.settings.classic_client_exe, None, automatic_label.as_str());
                    for path in &folder_exes {
                        let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
                        ui.selectable_value(&mut self.settings.classic_client_exe, Some(path.clone()), name);
                    }
                });
            if ui.button("Browse...").clicked() {
                if let Some(path) = uot_gui::open_filtered_file_dialog(
                    self.settings.cc_dir.as_deref(),
                    "Executable",
                    &["exe"],
                ) {
                    self.settings.classic_client_exe = Some(path);
                }
            }
            if ui.button("Re-check").clicked() {
                let repaint_ctx = ui.ctx().clone();
                self.dif_check.recheck(move || repaint_ctx.request_repaint());
            }
            if self.dif_check.is_running() {
                ui.spinner();
            }
        });

        let Some(report) = &self.dif_check.report else {
            return;
        };
        ui.horizontal(|ui| {
            ui.add_space(DIF_CHECK_INDENT);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(exe_line(&report.exe, &report.folder_exes)).weak());
                match report.result.as_deref() {
                    None => {}
                    Some(Err(error)) => {
                        ui.label(format!("Cannot check the executable: {error}"));
                    }
                    Some(Ok(usage)) => {
                        for kind in DifKind::ALL {
                            let status = usage.status(kind);
                            ui.label(status_line(kind, status)).on_hover_text(status_detail(kind, status));
                        }
                        egui::CollapsingHeader::new("Dif check evidence").id_salt("dif_check_evidence").show(ui, |ui| {
                            ui.label(egui::RichText::new(usage.advice()).weak());
                            for line in &usage.evidence {
                                ui.label(egui::RichText::new(line.as_str()).weak());
                            }
                        });
                    }
                }
            });
        });
    }

    pub fn show_preview(&mut self, path: &std::path::Path) {
        self.preview_path = Some(path.to_path_buf());
        self.preview_texture = None;
    }
}
