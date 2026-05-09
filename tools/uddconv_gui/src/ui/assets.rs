use eframe::egui;
use crate::app::UddConvApp;
use crate::ui::widgets::draw_asset_row;

impl UddConvApp {
    pub fn ui_assets(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.heading("Asset Packing");
            ui.label("Convert basic client assets into UODynamapper optimized packages.");
            ui.add_space(15.0);

            let is_busy = *self.is_converting.lock().unwrap();

            ui.add_enabled_ui(!is_busy, |ui| {
                ui.spacing_mut().item_spacing.y = 15.0;

                if draw_asset_row(
                    ui,
                    "Pack CC Art",
                    "Classic items and land textures (art.mul)",
                    Some(&mut self.settings.opt_cc_art),
                ) {
                    self.convert_cc_art();
                }
                if draw_asset_row(
                    ui,
                    "Pack EC Art",
                    "Enhanced Client static items (worldart)",
                    Some(&mut self.settings.opt_ec_art),
                ) {
                    self.convert_ec_art();
                }
                if draw_asset_row(
                    ui,
                    "Pack EC Land",
                    "Enhanced Client high-res terrain textures",
                    Some(&mut self.settings.opt_ec_land),
                ) {
                    self.convert_ec_land();
                }

                ui.indent("ec_land_upscale", |ui| {
                    ui.collapsing("EC Land Upscaling Options", |ui| {
                        ui.label("Upscale low-resolution EC terrain textures to improve quality.");
                        ui.add_space(5.0);
                        crate::ui::widgets::draw_upscale_config(ui, "Source 64x64  ", &mut self.settings.upscale_ec_land_64);
                        crate::ui::widgets::draw_upscale_config(ui, "Source 128x128", &mut self.settings.upscale_ec_land_128);
                        crate::ui::widgets::draw_upscale_config(ui, "Source 256x256", &mut self.settings.upscale_ec_land_256);
                        ui.add_space(5.0);
                    });
                });
                if draw_asset_row(
                    ui,
                    "Pack Tilemeta",
                    "Unified metadata and radar color data",
                    None,
                ) {
                    self.convert_tilemeta();
                }
            });
        });
    }
}
