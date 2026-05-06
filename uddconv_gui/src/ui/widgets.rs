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
                        ui.radio_value(opt_val, TextureOptimization::Planar, "Planar (Disk)")
                            .on_hover_text("De-interleaved channels. Better compression ratio for lossless storage.");
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
