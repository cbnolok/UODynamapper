use crate::app::UddConvApp;
use crate::models::TextureOptimization;
use udd_conv::upscale::UpscaleFilter;
use eframe::egui;

impl UddConvApp {
    pub fn ui_assets(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.heading("Asset Packing");
            ui.label("Convert basic client assets into UODynamapper optimized packages.");
            ui.add_space(20.0);

            ui.group(|ui| {
                ui.label(egui::RichText::new("Compression guidelines:").strong().color(egui::Color32::from_rgb(100, 200, 255)));
                ui.add_space(5.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("BC7:").strong().color(egui::Color32::from_rgb(255, 200, 100)));
                    ui.label("Lossy but reduces disk size and VRAM usage. Sampled directly by the GPU in its compressed state.");
                });
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("Supercompressed BC7:").strong().color(egui::Color32::from_rgb(255, 200, 100)));
                    ui.label("BC7 data compressed with zstd. Extremely small disk footprint, but requires on-the-fly decompression when loading.");
                });
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("Jpeg XL:").strong().color(egui::Color32::from_rgb(255, 200, 100)));
                    ui.label("Lossless and reduces disk size, but offers no VRAM savings.");
                });
            });
            ui.add_space(20.0);

            let is_busy = *self.is_converting.lock().unwrap();

            ui.add_enabled_ui(!is_busy, |ui| {
                ui.spacing_mut().item_spacing.y = 15.0;

                if draw_asset_row(
                    ui,
                    "Pack CC Art",
                    "Classic items and land textures (art.mul)",
                    Some(&mut self.settings.opt_cc_art),
                    Some(&mut self.settings.upscale_cc_art),
                ) {
                    self.convert_cc_art();
                }
                if draw_asset_row(
                    ui,
                    "Pack CC Texmaps",
                    "Classic high-res terrain textures (texmaps.mul)",
                    Some(&mut self.settings.opt_cc_texmaps),
                    Some(&mut self.settings.upscale_cc_texmaps),
                ) {
                    self.convert_cc_texmaps();
                }
                if draw_asset_row(
                    ui,
                    "Pack EC Art",
                    "Enhanced Client static items (worldart)",
                    Some(&mut self.settings.opt_ec_art),
                    Some(&mut self.settings.upscale_ec_art),
                ) {
                    self.convert_ec_art();
                }
                if draw_asset_row(
                    ui,
                    "Pack EC Land",
                    "Enhanced Client high-res terrain textures",
                    Some(&mut self.settings.opt_ec_land),
                    Some(&mut self.settings.upscale_ec_land),
                ) {
                    self.convert_ec_land();
                }

                if draw_asset_row(
                    ui,
                    "Pack Tilemeta",
                    "Unified metadata and radar color data",
                    None,
                    None,
                ) {
                    self.convert_tilemeta();
                }
            });
        });
    }
}

fn draw_asset_row(
    ui: &mut egui::Ui,
    title: &str,
    desc: &str,
    opt: Option<&mut TextureOptimization>,
    upscale: Option<&mut UpscaleFilter>,
) -> bool {
    let mut clicked = false;
    let btn_size = egui::vec2(150.0, 40.0);

    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.add_space(5.0);
                if ui
                    .add_sized(btn_size, egui::Button::new(egui::RichText::new(title).strong()))
                    .clicked()
                {
                    clicked = true;
                }
            });

            ui.add_space(20.0);

            if opt.is_some() || upscale.is_some() {
                ui.vertical(|ui| {
                    if let Some(opt_val) = opt {
                        let current_fmt = match opt_val {
                            TextureOptimization::None => "Raw+zstd (default)",
                            TextureOptimization::Bc7 => "BC7",
                            TextureOptimization::Bc7Zstd => "Supercompressed BC7+zstd",
                            TextureOptimization::JpegXl => "Jpeg XL",
                        };

                        egui::ComboBox::from_id_salt(format!("{}_fmt", title))
                            .selected_text(current_fmt)
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(current_fmt == "Raw+zstd (default)", "Raw+zstd (default)").clicked() {
                                    *opt_val = TextureOptimization::None;
                                }
                                if ui.selectable_label(current_fmt == "BC7", "BC7").clicked() {
                                    *opt_val = TextureOptimization::Bc7;
                                }
                                if ui.selectable_label(current_fmt == "Supercompressed BC7+zstd", "Supercompressed BC7+zstd").clicked() {
                                    *opt_val = TextureOptimization::Bc7Zstd;
                                }
                                if ui.selectable_label(current_fmt == "Jpeg XL", "Jpeg XL").clicked() {
                                    *opt_val = TextureOptimization::JpegXl;
                                }
                            });
                    }

                    if let Some(up_val) = upscale {
                        ui.add_space(5.0);
                        let current_up = format!("{:?}", up_val);
                        egui::ComboBox::from_id_salt(format!("{}_upscale", title))
                            .selected_text(if matches!(up_val, UpscaleFilter::None) { "No Upscaling" } else { &current_up })
                            .width(220.0)
                            .show_ui(ui, |ui| {
                                let filters = [
                                    UpscaleFilter::None,
                                    UpscaleFilter::Nearest,
                                    UpscaleFilter::Bilinear,
                                    UpscaleFilter::CatmullRom,
                                    UpscaleFilter::Lanczos3,
                                    UpscaleFilter::Lq2x,
                                    UpscaleFilter::Lq3x,
                                    UpscaleFilter::Lq4x,
                                    UpscaleFilter::SuperSai,
                                    UpscaleFilter::FsrEasu,
                                    UpscaleFilter::FsrEasuRcas,
                                    UpscaleFilter::Depixelize2x,
                                    UpscaleFilter::Depixelize3x,
                                    UpscaleFilter::Depixelize4x,
                                    UpscaleFilter::Nedi,
                                    UpscaleFilter::TwoSai,
                                    UpscaleFilter::SuperEagle,
                                    UpscaleFilter::Hq2x,
                                    UpscaleFilter::Hq3x,
                                    UpscaleFilter::Hq4x,
                                    UpscaleFilter::Epx,
                                    UpscaleFilter::Epx3x,
                                    UpscaleFilter::Epx4x,
                                    UpscaleFilter::Xbr,
                                ];
                                for f in filters {
                                    let label = if matches!(f, UpscaleFilter::None) { "No Upscaling".to_string() } else { format!("{:?}", f) };
                                    if ui.selectable_label(*up_val == f, label).clicked() {
                                        *up_val = f;
                                    }
                                }
                            });
                    }
                });
                
                ui.add_space(20.0);
            }

            ui.vertical(|ui| {
                ui.add_space(2.0);
                ui.label(egui::RichText::new(title).strong().size(18.0));
                ui.add_space(2.0);
                ui.label(desc);
            });
        });
    });

    clicked
}
