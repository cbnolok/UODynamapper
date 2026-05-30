use crate::app::UddConvApp;
use crate::models::{AssetPackProgress, AssetPackProgressState, AssetPackTask, TextureOptimization};
use eframe::egui;
use udd_conv::source_paths::gather_source_dirs;
use udd_conv::tex_art_cc::select_tex_art_cc_metadata_source;
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
            if is_busy {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }

            draw_section_header(ui, "Classic Client", "Assets read from the Classic Client directory");
            let tex_art_cc_metadata_source = tex_art_cc_metadata_source_label(self);
            let tex_art_cc_progress = self.asset_pack_progress(AssetPackTask::TexArtCc);
            let (clicked, stop_clicked, preview) = draw_asset_card(
                ui,
                "Classic Art",
                "Items and land textures from art.mul",
                Some(&tex_art_cc_metadata_source),
                &tex_art_cc_progress,
                !is_busy,
                Some((&mut self.settings.zstd_tex_art_cc, &mut self.settings.jxl_tex_art_cc)),
                None,
                Some(&mut self.settings.opt_tex_art_cc),
                Some(&mut self.settings.bc7_rdo_lambda),
                Some((&mut self.settings.upscale_tex_art_cc, crate::models::UpscalePreviewTarget::TexArtCc)),
                vec![],
            );
            if clicked { self.convert_tex_art_cc(); }
            if stop_clicked { self.request_cancel_conversion(); }
            if let Some((target, filter)) = preview { self.open_upscale_preview(target, filter); }

            let tex_land_cc_progress = self.asset_pack_progress(AssetPackTask::TexLandCc);
            let (clicked, stop_clicked, preview) = draw_asset_card(
                ui,
                "Classic Texmaps",
                "High-resolution terrain textures from texmaps.mul",
                None,
                &tex_land_cc_progress,
                !is_busy,
                Some((&mut self.settings.zstd_tex_land_cc, &mut self.settings.jxl_tex_land_cc)),
                None,
                Some(&mut self.settings.opt_tex_land_cc),
                Some(&mut self.settings.bc7_rdo_lambda),
                None,
                vec![
                    ("64x64", &mut self.settings.upscale_tex_land_cc_64, crate::models::UpscalePreviewTarget::TexLandCc64),
                    ("128x128", &mut self.settings.upscale_tex_land_cc_128, crate::models::UpscalePreviewTarget::TexLandCc128),
                ],
            );
            if clicked { self.convert_tex_land_cc(); }
            if stop_clicked { self.request_cancel_conversion(); }
            if let Some((target, filter)) = preview { self.open_upscale_preview(target, filter); }

            ui.add_space(4.0);
            draw_section_header(ui, "Enhanced Client", "Assets read from the Enhanced Client directory");
            let tex_art_ec_progress = self.asset_pack_progress(AssetPackTask::TexArtEc);
            let (clicked, stop_clicked, preview) = draw_asset_card(
                ui,
                "Enhanced Art",
                "Static items from worldart",
                None,
                &tex_art_ec_progress,
                !is_busy,
                Some((&mut self.settings.zstd_tex_art_ec, &mut self.settings.jxl_tex_art_ec)),
                None,
                Some(&mut self.settings.opt_tex_art_ec),
                Some(&mut self.settings.bc7_rdo_lambda),
                Some((&mut self.settings.upscale_tex_art_ec, crate::models::UpscalePreviewTarget::TexArtEc)),
                vec![],
            );
            if clicked { self.convert_tex_art_ec(); }
            if stop_clicked { self.request_cancel_conversion(); }
            if let Some((target, filter)) = preview { self.open_upscale_preview(target, filter); }

            let tex_land_ec_progress = self.asset_pack_progress(AssetPackTask::TexLandEc);
            let (clicked, stop_clicked, preview) = draw_asset_card(
                ui,
                "Enhanced Land",
                "High-resolution terrain textures",
                None,
                &tex_land_ec_progress,
                !is_busy,
                Some((&mut self.settings.zstd_tex_land_ec, &mut self.settings.jxl_tex_land_ec)),
                None,
                Some(&mut self.settings.opt_tex_land_ec),
                Some(&mut self.settings.bc7_rdo_lambda),
                None,
                vec![
                    ("64x64", &mut self.settings.upscale_tex_land_ec_64, crate::models::UpscalePreviewTarget::TexLandEc64),
                    ("128x128", &mut self.settings.upscale_tex_land_ec_128, crate::models::UpscalePreviewTarget::TexLandEc128),
                    ("256x256", &mut self.settings.upscale_tex_land_ec_256, crate::models::UpscalePreviewTarget::TexLandEc256),
                    ("512x512", &mut self.settings.upscale_tex_land_ec_512, crate::models::UpscalePreviewTarget::TexLandEc512),
                ],
            );
            if clicked { self.convert_tex_land_ec(); }
            if stop_clicked { self.request_cancel_conversion(); }
            if let Some((target, filter)) = preview { self.open_upscale_preview(target, filter); }

            ui.add_space(4.0);
            draw_section_header(ui, "Shared Metadata", "Classic tiledata with Enhanced tileart and string dictionary data");
            let tilemeta_progress = self.asset_pack_progress(AssetPackTask::TileMeta);
            let (clicked, stop_clicked, _) = draw_asset_card(
                ui,
                "Tile Metadata",
                "Unified metadata and radar color data",
                None,
                &tilemeta_progress,
                !is_busy,
                None,
                Some(&mut self.settings.zstd_tilemeta),
                None,
                None,
                None,
                vec![],
            );
            if clicked {
                self.convert_tilemeta();
            }
            if stop_clicked { self.request_cancel_conversion(); }
        });
    }
}

fn tex_art_cc_metadata_source_label(app: &UddConvApp) -> String {
    let sources = gather_source_dirs(app.settings.cc_dir.as_ref(), app.settings.ec_dir.as_ref());
    if sources.is_empty() {
        return "Draw offsets: select a Classic or Enhanced source directory".to_string();
    }

    match select_tex_art_cc_metadata_source(&sources) {
        Ok(source) => format!("Draw offsets: {}", source.path.display()),
        Err(_) => "Draw offsets: missing tiledata.mul or tileart.uop".to_string(),
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
                    ui.label(egui::RichText::new("BC7 outputs use automatic 4x4-aligned atlas placement.").weak());
                    ui.label(egui::RichText::new("Land textures always include filtering-aware gutters.").weak());
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
    source_note: Option<&str>,
    progress: &AssetPackProgress,
    can_start: bool,
    texture_levels: Option<(&mut i32, &mut u8)>,
    zstd_only_level: Option<&mut i32>,
    opt: Option<&mut TextureOptimization>,
    bc7_rdo_lambda: Option<&mut f32>,
    upscale_single: Option<(&mut UpscaleFilter, crate::models::UpscalePreviewTarget)>,
    mut upscale_configs: Vec<(&str, &mut udd_conv::upscale::UpscaleConfig, crate::models::UpscalePreviewTarget)>,
) -> (bool, bool, Option<(crate::models::UpscalePreviewTarget, UpscaleFilter)>) {
    let mut clicked = false;
    let mut stop_clicked = false;
    let mut preview_req = None;
    let is_running = progress.state == AssetPackProgressState::Running;

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
                        ui.set_width(ui.available_width().min(420.0).max(180.0));
                        ui.label(
                            egui::RichText::new(title)
                                .strong()
                                .size(18.0)
                                .color(egui::Color32::WHITE),
                        );
                        ui.label(egui::RichText::new(desc).size(12.5).weak());
                        if let Some(source_note) = source_note {
                            ui.label(
                                egui::RichText::new(source_note)
                                    .size(12.5)
                                    .color(egui::Color32::from_rgb(100, 200, 255)),
                            );
                        }
                    });

                    ui.vertical(|ui| {
                        ui.set_width((ui.available_width() - 132.0).max(180.0));
                        draw_asset_progress(ui, progress);
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if is_running {
                            if ui
                                .add_sized(
                                    [90.0, 30.0],
                                    egui::Button::new(egui::RichText::new("Stop").strong()),
                                )
                                .on_hover_text("Request conversion stop")
                                .clicked()
                            {
                                stop_clicked = true;
                            }
                        } else if ui
                            .add_enabled(
                                can_start,
                                egui::Button::new(egui::RichText::new("Pack Asset").strong()),
                            )
                            .on_hover_text(if can_start { "Pack this asset" } else { "Another task is running" })
                            .clicked()
                        {
                            clicked = true;
                        }
                    });
                });

                if opt.is_some()
                    || texture_levels.is_some()
                    || zstd_only_level.is_some()
                    || bc7_rdo_lambda.is_some()
                    || upscale_single.is_some()
                    || !upscale_configs.is_empty()
                {
                    ui.add_space(2.0);
                    ui.separator();
                    ui.add_space(2.0);

                    ui.add_enabled_ui(can_start, |ui| {
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
                                            )
                                                .on_hover_text("0 disables RDO. Higher lambda values usually improve BC7+zstd disk ratio but can be much slower and may add more loss.");
                                        });
                                    }
                                }

                                if let Some((zstd_level, jxl_level)) = texture_levels {
                                    match *opt_val {
                                        TextureOptimization::None | TextureOptimization::Bc7Zstd => {
                                            draw_zstd_level_cell(ui, zstd_level);
                                        }
                                        TextureOptimization::JpegXl => {
                                            draw_jxl_level_cell(ui, jxl_level);
                                        }
                                        TextureOptimization::Bc7 => {}
                                    }
                                }
                            } else if let Some(zstd_level) = zstd_only_level {
                                draw_zstd_level_cell(ui, zstd_level);
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
                    });
                }
            });
        });

    (clicked, stop_clicked, preview_req)
}

fn draw_asset_progress(ui: &mut egui::Ui, progress: &AssetPackProgress) {
    let text = match progress.state {
        AssetPackProgressState::Idle => egui::RichText::new(&progress.text).weak(),
        AssetPackProgressState::Running => {
            egui::RichText::new(&progress.text).color(egui::Color32::from_rgb(110, 205, 255))
        }
        AssetPackProgressState::Succeeded => {
            egui::RichText::new(&progress.text).color(egui::Color32::from_rgb(120, 220, 150))
        }
        AssetPackProgressState::Cancelled => {
            egui::RichText::new(&progress.text).color(egui::Color32::from_rgb(255, 190, 110))
        }
        AssetPackProgressState::Failed => {
            egui::RichText::new(&progress.text).color(egui::Color32::from_rgb(255, 120, 120))
        }
    };
    ui.add(
        egui::ProgressBar::new(progress.fraction)
            .animate(progress.state == AssetPackProgressState::Running)
            .text(text),
    );
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

fn draw_zstd_level_cell(ui: &mut egui::Ui, level: &mut i32) {
    draw_control_cell(ui, "Zstd Level", 96.0, |ui| {
        ui.add(
            egui::DragValue::new(level)
                .speed(1)
                .range(1..=22),
        )
            .on_hover_text("Lower levels are faster and larger. Level 7 is the balanced default; 19-22 can be much slower for smaller packages.");
    });
}

fn draw_jxl_level_cell(ui: &mut egui::Ui, level: &mut u8) {
    draw_control_cell(ui, "JPEG XL Level", 116.0, |ui| {
        ui.add(
            egui::DragValue::new(level)
                .speed(1)
                .range(1..=10),
        )
            .on_hover_text("Lower levels are faster and larger. Level 6 is the balanced default; 9-10 can be much slower for smaller packages.");
    });
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
