use crate::app::UddConvApp;
use crate::models::{AtlasPackingModeSetting, TextureOptimization};
use eframe::egui;
use udd_conv::upscale::UpscaleFilter;

impl UddConvApp {
    pub fn ui_assets(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);

            ui.heading("Asset Packing");
            ui.label("Convert basic client assets into UODynamapper optimized packages.");
            ui.add_space(6.0);
            draw_assets_overview(ui, self);
            ui.add_space(2.0);

            let is_busy = self.is_busy();

            ui.add_enabled_ui(!is_busy, |ui| {
                draw_section_header(ui, "Classic Client", "Assets read from the Classic Client directory");
                let (clicked, preview) = draw_asset_card(
                    ui,
                    "Classic Art",
                    "Items and land textures from art.mul",
                    Some(&mut self.settings.opt_tex_art_cc),
                    Some(&mut self.settings.bc7_rdo_lambda),
                    Some(&mut self.settings.packing_tex_art_cc),
                    Some(&mut self.settings.filtering_ready_tex_art_cc),
                    Some((&mut self.settings.upscale_tex_art_cc, crate::models::UpscalePreviewTarget::TexArtCc)),
                    vec![],
                );
                if clicked { self.convert_tex_art_cc(); }
                if let Some((target, filter)) = preview { self.open_upscale_preview(target, filter); }

                let (clicked, preview) = draw_asset_card(
                    ui,
                    "Classic Texmaps",
                    "High-resolution terrain textures from texmaps.mul",
                    Some(&mut self.settings.opt_tex_land_cc),
                    Some(&mut self.settings.bc7_rdo_lambda),
                    Some(&mut self.settings.packing_tex_land_cc),
                    Some(&mut self.settings.filtering_ready_tex_land_cc),
                    None,
                    vec![
                        ("64x64", &mut self.settings.upscale_tex_land_cc_64, crate::models::UpscalePreviewTarget::TexLandCc64),
                        ("128x128", &mut self.settings.upscale_tex_land_cc_128, crate::models::UpscalePreviewTarget::TexLandCc128),
                    ],
                );
                if clicked { self.convert_tex_land_cc(); }
                if let Some((target, filter)) = preview { self.open_upscale_preview(target, filter); }

                ui.add_space(4.0);
                draw_section_header(ui, "Enhanced Client", "Assets read from the Enhanced Client directory");
                let (clicked, preview) = draw_asset_card(
                    ui,
                    "Enhanced Art",
                    "Static items from worldart",
                    Some(&mut self.settings.opt_tex_art_ec),
                    Some(&mut self.settings.bc7_rdo_lambda),
                    Some(&mut self.settings.packing_tex_art_ec),
                    Some(&mut self.settings.filtering_ready_tex_art_ec),
                    Some((&mut self.settings.upscale_tex_art_ec, crate::models::UpscalePreviewTarget::TexArtEc)),
                    vec![],
                );
                if clicked { self.convert_tex_art_ec(); }
                if let Some((target, filter)) = preview { self.open_upscale_preview(target, filter); }

                let (clicked, preview) = draw_asset_card(
                    ui,
                    "Enhanced Land",
                    "High-resolution terrain textures",
                    Some(&mut self.settings.opt_tex_land_ec),
                    Some(&mut self.settings.bc7_rdo_lambda),
                    Some(&mut self.settings.packing_tex_land_ec),
                    Some(&mut self.settings.filtering_ready_tex_land_ec),
                    None,
                    vec![
                        ("64x64", &mut self.settings.upscale_tex_land_ec_64, crate::models::UpscalePreviewTarget::TexLandEc64),
                        ("128x128", &mut self.settings.upscale_tex_land_ec_128, crate::models::UpscalePreviewTarget::TexLandEc128),
                        ("256x256", &mut self.settings.upscale_tex_land_ec_256, crate::models::UpscalePreviewTarget::TexLandEc256),
                        ("512x512", &mut self.settings.upscale_tex_land_ec_512, crate::models::UpscalePreviewTarget::TexLandEc512),
                    ],
                );
                if clicked { self.convert_tex_land_ec(); }
                if let Some((target, filter)) = preview { self.open_upscale_preview(target, filter); }

                ui.add_space(4.0);
                draw_section_header(ui, "Shared Metadata", "Classic tiledata with Enhanced tileart and string dictionary data");
                if draw_asset_card(
                    ui,
                    "Tile Metadata",
                    "Unified metadata and radar color data",
                    None,
                    None,
                    None,
                    None,
                    None,
                    vec![],
                ).0 {
                    self.convert_tilemeta();
                }
            });
        });
    }
}

fn draw_assets_overview(ui: &mut egui::Ui, app: &mut UddConvApp) {
    egui::Frame::group(ui.style())
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(6.0)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(16.0, 6.0);
                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new("Classic patch files")
                            .strong()
                            .color(egui::Color32::from_rgb(255, 210, 120)),
                    );
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut app.settings.include_verdata, "verdata.mul");
                        ui.checkbox(&mut app.settings.include_map_difs, "map difs");
                        ui.checkbox(&mut app.settings.include_static_difs, "static difs");
                    });
                });

                ui.separator();

                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new("Compression")
                            .strong()
                            .color(egui::Color32::from_rgb(100, 200, 255)),
                    );
                    ui.label(egui::RichText::new("BC7 saves VRAM. BC7+zstd saves disk. Jpeg XL is lossless.").weak());
                    ui.label(egui::RichText::new("BC7-oriented packing keeps atlas placements block-aligned.").weak());
                });
            });
        });
}

fn draw_section_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title)
                .strong()
                .size(16.0)
                .color(egui::Color32::from_rgb(230, 230, 230)),
        );
        ui.label(egui::RichText::new(subtitle).weak());
    });
}

fn draw_asset_card(
    ui: &mut egui::Ui,
    title: &str,
    desc: &str,
    opt: Option<&mut TextureOptimization>,
    bc7_rdo_lambda: Option<&mut f32>,
    packing_mode: Option<&mut AtlasPackingModeSetting>,
    filtering_ready: Option<&mut bool>,
    upscale_single: Option<(&mut UpscaleFilter, crate::models::UpscalePreviewTarget)>,
    mut upscale_configs: Vec<(&str, &mut udd_conv::upscale::UpscaleConfig, crate::models::UpscalePreviewTarget)>,
) -> (bool, Option<(crate::models::UpscalePreviewTarget, UpscaleFilter)>) {
    let mut clicked = false;
    let mut preview_req = None;

    egui::Frame::group(ui.style())
        .fill(ui.visuals().widgets.noninteractive.bg_fill)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);

                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width((ui.available_width() - 132.0).max(180.0));
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
                                [120.0, 30.0],
                                egui::Button::new(egui::RichText::new("Pack Asset").strong()),
                            )
                            .clicked()
                        {
                            clicked = true;
                        }
                    });
                });

                if opt.is_some() || bc7_rdo_lambda.is_some() || filtering_ready.is_some() || upscale_single.is_some() || !upscale_configs.is_empty() {
                    ui.add_space(2.0);
                    ui.separator();
                    ui.add_space(2.0);

                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(12.0, 8.0);

                        if let Some(opt_val) = opt {
                            let show_bc7_settings = matches!(opt_val, TextureOptimization::Bc7 | TextureOptimization::Bc7Zstd);
                            draw_control_cell(ui, "Output Format", 210.0, |ui| {
                                let current_fmt = match opt_val {
                                    TextureOptimization::None => "Raw+zstd (default)",
                                    TextureOptimization::Bc7 => "BC7",
                                    TextureOptimization::Bc7Zstd => "Supercompressed BC7+zstd",
                                    TextureOptimization::JpegXl => "Jpeg XL",
                                };

                                egui::ComboBox::from_id_salt(format!("{}_fmt", title))
                                    .selected_text(current_fmt)
                                    .width(ui.available_width())
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

                            if show_bc7_settings {
                                if let Some(lambda) = bc7_rdo_lambda {
                                    draw_control_cell(ui, "BC7 RDO", 96.0, |ui| {
                                        ui.add(
                                            egui::DragValue::new(lambda)
                                                .speed(0.01)
                                                .range(0.0..=1.0),
                                        );
                                    });
                                }
                            }
                        }

                        if let Some(mode_val) = packing_mode {
                            draw_control_cell(ui, "Packing Mode", 190.0, |ui| {
                                let current_mode = match mode_val {
                                    AtlasPackingModeSetting::MaximumPacking => "Maximum packing",
                                    AtlasPackingModeSetting::Bc7Oriented => "BC7-oriented",
                                };

                                egui::ComboBox::from_id_salt(format!("{}_packing", title))
                                    .selected_text(current_mode)
                                    .width(ui.available_width())
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
                        }

                        if let Some(filtering_ready_val) = filtering_ready {
                            draw_control_cell(ui, "Sampling", 178.0, |ui| {
                                ui.checkbox(filtering_ready_val, "Filtering-ready gutters");
                            });
                        }

                        if let Some((up_val, target)) = upscale_single {
                            draw_control_cell(ui, "Upscale Filter", 206.0, |ui| {
                                ui.horizontal(|ui| {
                                    ui.set_width(ui.available_width());
                                    let combo_width = (ui.available_width() - 34.0).max(120.0);
                                    ui.scope(|ui| {
                                        ui.set_width(combo_width);
                                        draw_upscale_filter(ui, format!("{}_upscale", title), up_val);
                                    });
                                    if ui.button("🔍").on_hover_text("Preview").clicked() {
                                        preview_req = Some((target, *up_val));
                                    }
                                });
                            });
                        }
                    });

                    if !upscale_configs.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(12.0, 8.0);
                            for (label, config, target) in upscale_configs.iter_mut() {
                                draw_control_cell(ui, &format!("Upscale {}", label), 190.0, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.set_width(ui.available_width());
                                        let combo_width = (ui.available_width() - 34.0).max(120.0);
                                        ui.scope(|ui| {
                                            ui.set_width(combo_width);
                                            draw_upscale_filter(
                                                ui,
                                                format!("{}_upscale_{}", title, label),
                                                &mut config.filter,
                                            );
                                        });
                                        if ui.button("🔍").on_hover_text("Preview").clicked() {
                                            preview_req = Some((*target, config.filter));
                                        }
                                    });
                                });
                            }
                        });
                    }
                }
            });
        });

    (clicked, preview_req)
}

fn draw_control_cell<R>(
    ui: &mut egui::Ui,
    label: &str,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.vertical(|ui| {
        ui.set_width(width);
        ui.label(egui::RichText::new(label).size(12.0).weak());
        add_contents(ui)
    })
    .inner
}

fn draw_upscale_filter(ui: &mut egui::Ui, id: String, up_val: &mut UpscaleFilter) {
    let current_up = format!("{:?}", up_val);
    egui::ComboBox::from_id_salt(id)
        .selected_text(if matches!(up_val, UpscaleFilter::None) {
            "No Upscaling"
        } else {
            &current_up
        })
        .width(ui.available_width())
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
                UpscaleFilter::KLDepixelize2x,
                UpscaleFilter::KLDepixelize3x,
                UpscaleFilter::KLDepixelize4x,
                UpscaleFilter::Nedi2x,
                UpscaleFilter::TwoSai2x,
                UpscaleFilter::SuperEagle2x,
                UpscaleFilter::Hq2xSimple,
                UpscaleFilter::Hq3xSimple,
                UpscaleFilter::Hq4xSimple,
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
