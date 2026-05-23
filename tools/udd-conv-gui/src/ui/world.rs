use crate::app::UddConvApp;
use eframe::egui;
use udd_conv::cc_radar::RadarFormat;

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
            });

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

    pub fn show_preview(&mut self, path: &std::path::Path) {
        self.preview_path = Some(path.to_path_buf());
        self.preview_texture = None;
    }
}
