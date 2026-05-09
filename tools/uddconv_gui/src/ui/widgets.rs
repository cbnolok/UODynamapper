use eframe::egui;

use crate::models::TextureOptimization;

pub fn draw_asset_row(
    ui: &mut egui::Ui,
    title: &str,
    desc: &str,
    mut opt: Option<&mut TextureOptimization>,
) -> bool {
    let row_width = ui.available_width().min(700.0);
    let btn_size = egui::vec2(160.0, 40.0);
    let mut clicked = false;

    ui.group(|ui| {
        ui.set_width(row_width);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                if ui.add_sized(btn_size, egui::Button::new(egui::RichText::new(title).strong())).clicked() {
                    clicked = true;
                }
                if let Some(opt_val) = opt.as_deref_mut() {
                    ui.horizontal(|ui| {
                        ui.radio_value(opt_val, TextureOptimization::None, "Raw");
                        ui.radio_value(opt_val, TextureOptimization::Bc7, "BC7 (VRAM)")
                            .on_hover_text("GPU-compressed textures. Faster rendering, less VRAM, lossy quality.");
                        ui.radio_value(opt_val, TextureOptimization::JpegXl, "Jpeg XL (Best)")
                            .on_hover_text("State-of-the-art lossless image compression. Best ratio, slower to build.");
                    });
                }
            });

            ui.add_space(20.0);

            ui.vertical(|ui| {
                ui.label(egui::RichText::new(title).strong().size(18.0));
                ui.add_space(4.0);
                ui.label(desc);
            });
        });
    });

    clicked
}
pub fn draw_upscale_config(
    ui: &mut egui::Ui,
    label: &str,
    config: &mut uddconv::upscale::UpscaleConfig,
) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).strong());
        ui.add_space(10.0);

        ui.label("Target:");
        egui::ComboBox::from_id_salt(format!("{}_size", label))
            .selected_text(format!("{}x{}", config.target_size, config.target_size))
            .width(80.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut config.target_size, 64, "64x64");
                ui.selectable_value(&mut config.target_size, 128, "128x128");
                ui.selectable_value(&mut config.target_size, 256, "256x256");
                ui.selectable_value(&mut config.target_size, 512, "512x512");
            });

        ui.add_space(10.0);
        ui.label("Algorithm:");
        egui::ComboBox::from_id_salt(format!("{}_algo", label))
            .selected_text(format!("{:?}", config.filter))
            .width(120.0)
            .show_ui(ui, |ui| {
                use uddconv::upscale::UpscaleFilter::*;
                ui.selectable_value(&mut config.filter, None, "None");
                ui.selectable_value(&mut config.filter, Nearest, "Nearest");
                ui.selectable_value(&mut config.filter, Bilinear, "Bilinear");
                ui.selectable_value(&mut config.filter, CatmullRom, "Catmull-Rom");
                ui.selectable_value(&mut config.filter, Lanczos3, "Lanczos3");
                ui.selectable_value(&mut config.filter, FsrEasu, "FSR 1.1 (EASU)");
                ui.selectable_value(&mut config.filter, FsrEasuRcas, "FSR 1.1 (EASU + RCAS)");
                ui.selectable_value(&mut config.filter, Depixelize2x, "Depixelize 2x");
                ui.selectable_value(&mut config.filter, Depixelize3x, "Depixelize 3x");
                ui.selectable_value(&mut config.filter, Depixelize4x, "Depixelize 4x");
                ui.separator();
                ui.selectable_value(&mut config.filter, Epx, "EPX / Scale2x");
                ui.selectable_value(&mut config.filter, Epx3x, "EPX / Scale3x");
                ui.selectable_value(&mut config.filter, Epx4x, "EPX / Scale4x");
                ui.selectable_value(&mut config.filter, Xbr, "xBRZ");
                ui.selectable_value(&mut config.filter, Hq2x, "HQ2x");
                ui.selectable_value(&mut config.filter, Hq3x, "HQ3x");
                ui.selectable_value(&mut config.filter, Hq4x, "HQ4x");
                ui.selectable_value(&mut config.filter, TwoSai, "2xSaI");
                ui.selectable_value(&mut config.filter, SuperSai, "Super 2xSaI");
                ui.selectable_value(&mut config.filter, SuperEagle, "SuperEagle");
                ui.selectable_value(&mut config.filter, Nedi, "NEDI");
                ui.selectable_value(&mut config.filter, Lq2x, "LQ2x");
                ui.selectable_value(&mut config.filter, Lq3x, "LQ3x");
                ui.selectable_value(&mut config.filter, Lq4x, "LQ4x");
            });
    });
}
