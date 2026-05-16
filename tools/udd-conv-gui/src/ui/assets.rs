use crate::app::UddConvApp;
use crate::models::{AtlasPackingModeSetting, TextureOptimization};
use eframe::egui;
use udd_conv::upscale::UpscaleFilter;

impl UddConvApp {
    pub fn ui_assets(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);

            ui.heading("Asset Packing");
            ui.label("Convert basic client assets into UODynamapper optimized packages.");
            ui.add_space(12.0);

            ui.group(|ui| {
                ui.label(egui::RichText::new("Compression guidelines:").strong().color(egui::Color32::from_rgb(100, 200, 255)));
                ui.add_space(5.0);
                egui::Grid::new("guidelines_grid")
                    .num_columns(2)
                    .spacing([15.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("BC7:").strong().color(egui::Color32::from_rgb(255, 200, 100)));
                        ui.label("Lossy but reduces disk size and VRAM usage. Sampled directly by the GPU in its compressed state.");
                        ui.end_row();

                        ui.label(egui::RichText::new("Supercompressed BC7:").strong().color(egui::Color32::from_rgb(255, 200, 100)));
                        ui.label("BC7 data compressed with zstd. Extremely small disk footprint, but requires on-the-fly decompression when loading.");
                        ui.end_row();

                        ui.label(egui::RichText::new("Jpeg XL:").strong().color(egui::Color32::from_rgb(255, 200, 100)));
                        ui.label("Lossless and reduces disk size, but offers no VRAM savings.");
                        ui.end_row();
                    });
            });
            ui.add_space(8.0);

            ui.group(|ui| {
                ui.label(
                    egui::RichText::new("Source selection and packing mode").strong().color(egui::Color32::from_rgb(255, 210, 120)),
                );
                ui.add_space(4.0);
                ui.label("Classic actions read only the CC client directory. Enhanced actions read only the EC client directory. Tile Metadata intentionally combines CC tiledata with EC tileart and string dictionary data.");
                ui.add_space(4.0);
                ui.label("Packing mode changes how atlas pages are laid out before encoding. Maximum packing uses all available space. BC7-oriented keeps placements BC7-friendly and block-aligned so the compressed output is easier to encode efficiently.");
            });
            ui.add_space(10.0);

            let is_busy = *self.is_converting.lock().unwrap();

            ui.add_enabled_ui(!is_busy, |ui| {
                let column_gap = 12.0;
                let card_width = ((ui.available_width() - column_gap).max(360.0)) * 0.5;

                ui.columns(2, |cols| {
                    cols[0].set_min_width(card_width);
                    cols[1].set_min_width(card_width);
                    cols[0].spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    cols[1].spacing_mut().item_spacing = egui::vec2(6.0, 6.0);

                    if draw_asset_card(
                        &mut cols[0],
                        "Classic Art",
                        "Classic items and land textures (art.mul)",
                        Some(&mut self.settings.opt_tex_art_cc),
                        Some(&mut self.settings.packing_tex_art_cc),
                        Some(&mut self.settings.upscale_tex_art_cc),
                        vec![],
                    ) {
                        self.convert_tex_art_cc();
                    }
                    if draw_asset_card(
                        &mut cols[1],
                        "Classic Texmaps",
                        "Classic high-res terrain textures (texmaps.mul)",
                        Some(&mut self.settings.opt_tex_land_cc),
                        Some(&mut self.settings.packing_tex_land_cc),
                        None,
                        vec![
                            ("64x64", &mut self.settings.upscale_tex_land_cc_64),
                            ("128x128", &mut self.settings.upscale_tex_land_cc_128),
                        ],
                    ) {
                        self.convert_tex_land_cc();
                    }
                });

                ui.add_space(8.0);

                ui.columns(2, |cols| {
                    cols[0].set_min_width(card_width);
                    cols[1].set_min_width(card_width);
                    cols[0].spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                    cols[1].spacing_mut().item_spacing = egui::vec2(6.0, 6.0);

                    if draw_asset_card(
                        &mut cols[0],
                        "Enhanced Art",
                        "Enhanced Client static items (worldart)",
                        Some(&mut self.settings.opt_tex_art_ec),
                        Some(&mut self.settings.packing_tex_art_ec),
                        Some(&mut self.settings.upscale_tex_art_ec),
                        vec![],
                    ) {
                        self.convert_tex_art_ec();
                    }
                    if draw_asset_card(
                        &mut cols[1],
                        "Enhanced Land",
                        "Enhanced Client high-res terrain textures",
                        Some(&mut self.settings.opt_tex_land_ec),
                        Some(&mut self.settings.packing_tex_land_ec),
                        None,
                        vec![
                            ("64x64", &mut self.settings.upscale_tex_land_ec_64),
                            ("128x128", &mut self.settings.upscale_tex_land_ec_128),
                            ("256x256", &mut self.settings.upscale_tex_land_ec_256),
                            ("512x512", &mut self.settings.upscale_tex_land_ec_512),
                        ],
                    ) {
                        self.convert_tex_land_ec();
                    }
                });

                ui.add_space(8.0);

                if draw_asset_card(
                    ui,
                    "Tile Metadata",
                    "Unified metadata and radar color data",
                    None,
                    None,
                    None,
                    vec![],
                ) {
                    self.convert_tilemeta();
                }
            });
        });
    }
}

fn draw_asset_card(
    ui: &mut egui::Ui,
    title: &str,
    desc: &str,
    opt: Option<&mut TextureOptimization>,
    packing_mode: Option<&mut AtlasPackingModeSetting>,
    upscale_single: Option<&mut UpscaleFilter>,
    mut upscale_configs: Vec<(&str, &mut udd_conv::upscale::UpscaleConfig)>,
) -> bool {
    let mut clicked = false;

    egui::Frame::group(ui.style())
        .fill(ui.visuals().widgets.noninteractive.bg_fill)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(5.0, 5.0);

                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new(title)
                                .strong()
                                .size(18.0)
                                .color(egui::Color32::WHITE),
                        );
                        ui.label(egui::RichText::new(desc).size(12.5).weak());
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_sized(
                                [112.0, 28.0],
                                egui::Button::new(egui::RichText::new("PACK ASSET").strong()),
                            )
                            .clicked()
                        {
                            clicked = true;
                        }
                    });
                });

                if opt.is_some() || upscale_single.is_some() || !upscale_configs.is_empty() {
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);

                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 5.0);

                        if let Some(opt_val) = opt {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Output Format:").size(12.5).weak());
                                let current_fmt = match opt_val {
                                    TextureOptimization::None => "Raw+zstd (default)",
                                    TextureOptimization::Bc7 => "BC7",
                                    TextureOptimization::Bc7Zstd => "Supercompressed BC7+zstd",
                                    TextureOptimization::JpegXl => "Jpeg XL",
                                };

                                egui::ComboBox::from_id_salt(format!("{}_fmt", title))
                                    .selected_text(current_fmt)
                                    .width(176.0)
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(
                                                current_fmt == "Raw+zstd (default)",
                                                "Raw+zstd (default)",
                                            )
                                            .clicked()
                                        {
                                            *opt_val = TextureOptimization::None;
                                        }
                                        if ui
                                            .selectable_label(current_fmt == "BC7", "BC7")
                                            .clicked()
                                        {
                                            *opt_val = TextureOptimization::Bc7;
                                        }
                                        if ui
                                            .selectable_label(
                                                current_fmt == "Supercompressed BC7+zstd",
                                                "Supercompressed BC7+zstd",
                                            )
                                            .clicked()
                                        {
                                            *opt_val = TextureOptimization::Bc7Zstd;
                                        }
                                        if ui
                                            .selectable_label(current_fmt == "Jpeg XL", "Jpeg XL")
                                            .clicked()
                                        {
                                            *opt_val = TextureOptimization::JpegXl;
                                        }
                                    });
                            });
                            ui.add_space(25.0);
                        }

                        if let Some(mode_val) = packing_mode {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Packing Mode:").size(12.5).weak());
                                let current_mode = match mode_val {
                                    AtlasPackingModeSetting::MaximumPacking => "Maximum packing",
                                    AtlasPackingModeSetting::Bc7Oriented => "BC7-oriented",
                                };

                                egui::ComboBox::from_id_salt(format!("{}_packing", title))
                                    .selected_text(current_mode)
                                    .width(176.0)
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(
                                                current_mode == "Maximum packing",
                                                "Maximum packing",
                                            )
                                            .clicked()
                                        {
                                            *mode_val = AtlasPackingModeSetting::MaximumPacking;
                                        }
                                        if ui
                                            .selectable_label(
                                                current_mode == "BC7-oriented",
                                                "BC7-oriented",
                                            )
                                            .clicked()
                                        {
                                            *mode_val = AtlasPackingModeSetting::Bc7Oriented;
                                        }
                                    });
                            });
                            ui.add_space(25.0);
                        }

                        if let Some(up_val) = upscale_single {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Upscale Filter:").size(12.5).weak());
                                draw_upscale_filter(ui, format!("{}_upscale", title), up_val);
                            });
                        }
                    });

                    if !upscale_configs.is_empty() {
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(8.0, 4.0);
                            for (label, config) in upscale_configs.iter_mut() {
                                ui.vertical(|ui| {
                                    ui.label(
                                        egui::RichText::new(format!("Upscale {}", label))
                                            .size(12.0)
                                            .weak(),
                                    );
                                    draw_upscale_filter(
                                        ui,
                                        format!("{}_upscale_{}", title, label),
                                        &mut config.filter,
                                    );
                                });
                                ui.add_space(15.0);
                            }
                        });
                    }
                }
            });
        });

    clicked
}

fn draw_upscale_filter(ui: &mut egui::Ui, id: String, up_val: &mut UpscaleFilter) {
    let current_up = format!("{:?}", up_val);
    egui::ComboBox::from_id_salt(id)
        .selected_text(if matches!(up_val, UpscaleFilter::None) {
            "No Upscaling"
        } else {
            &current_up
        })
        .width(144.0)
        .show_ui(ui, |ui| {
            let filters = [
                UpscaleFilter::None,
                UpscaleFilter::Nearest2x,
                UpscaleFilter::Nearest3x,
                UpscaleFilter::Nearest4x,
                UpscaleFilter::Bilinear2x,
                UpscaleFilter::Bilinear3x,
                UpscaleFilter::Bilinear4x,
                UpscaleFilter::CatmullRom2x,
                UpscaleFilter::CatmullRom3x,
                UpscaleFilter::CatmullRom4x,
                UpscaleFilter::Lanczos3_2x,
                UpscaleFilter::Lanczos3_3x,
                UpscaleFilter::Lanczos3_4x,
                UpscaleFilter::Lq2x,
                UpscaleFilter::Lq3x,
                UpscaleFilter::Lq4x,
                UpscaleFilter::SuperSai2x,
                UpscaleFilter::FsrEasu2x,
                UpscaleFilter::FsrEasu3x,
                UpscaleFilter::FsrEasu4x,
                UpscaleFilter::FsrEasuRcas2x,
                UpscaleFilter::FsrEasuRcas3x,
                UpscaleFilter::FsrEasuRcas4x,
                UpscaleFilter::Depixelize2x,
                UpscaleFilter::Depixelize3x,
                UpscaleFilter::Depixelize4x,
                UpscaleFilter::Nedi2x,
                UpscaleFilter::TwoSai2x,
                UpscaleFilter::SuperEagle2x,
                UpscaleFilter::Hq2x,
                UpscaleFilter::Hq3x,
                UpscaleFilter::Hq4x,
                UpscaleFilter::Hq2xTrue,
                UpscaleFilter::Hq3xTrue,
                UpscaleFilter::Hq4xTrue,
                UpscaleFilter::Epx2x,
                UpscaleFilter::Epx3x,
                UpscaleFilter::Epx4x,
                UpscaleFilter::Mmpx2x,
                UpscaleFilter::Mmpx4x,
                UpscaleFilter::Xbr2x,
                UpscaleFilter::Xbr3x,
                UpscaleFilter::Xbr4x,
            ];
            for f in filters {
                let label = if matches!(f, UpscaleFilter::None) {
                    "No Upscaling".to_string()
                } else {
                    format!("{:?}", f)
                };
                if ui.selectable_label(*up_val == f, label).clicked() {
                    *up_val = f;
                }
            }
        });
}
