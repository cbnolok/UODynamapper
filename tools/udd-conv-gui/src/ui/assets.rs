use crate::app::UddConvApp;
use crate::logic::tasks::{
    cc_gumps_source_dirs, ec_gumps_share, ec_gumps_source_dirs, effects_ec_source, hues_source,
    paperdoll_ec_sources, single_gump_pixel_format, EcGumpsShare, HuesSource, PAPERDOLL_EC_SOURCE_NAMES,
};
use crate::models::{
    AppSettings, AssetPackProgress, AssetPackProgressState, AssetPackTask, GumpsEcSharing, HueRamp,
    TextureOptimization, DEFAULT_ZSTD_LEVEL,
};
use crate::ui::VALIDATE_AFTER_PACKING_TOOLTIP;
use crate::ui::layout::{card_title, draw_card_grid, page_heading, section_heading, CardGrid};
use eframe::egui;
use udd_conv::cc_gumps::CC_LOSSY_SINGLE_GUMP_FORMAT;
use udd_conv::classic_sources::{resolve_classic_gump_source, SourceFormatPreference};
use udd_conv::ec_gumps::{find_ec_gumpart_dir, find_interface_uop, EC_LOSSY_SINGLE_GUMP_FORMAT};
use udd_conv::gump_atlas::{
    is_paperdoll_equipment_gump_id, PAPERDOLL_EQUIPMENT_GUMP_ID_END, PAPERDOLL_EQUIPMENT_GUMP_ID_START,
};
use udd_conv::page_compression::LOSSLESS_PAGE_ZSTD_LEVEL;
use udd_conv::single_gumps::{GumpPixelFormat, PAPERDOLL_BODY_GUMP_BY_BODY};
use udd_conv::source_paths::gather_source_dirs;
use udd_conv::tex_art_cc::select_tex_art_cc_metadata_source;
use udd_conv::tex_land_cc::CLASSIC_LAND_LOSSY_REFUSAL;
use udd_conv::upscale::UpscaleFilter;
use uocf::enhanced::effect_texture::EFFECT_TEXTURE_UOP_NAME;

impl UddConvApp {
    pub fn ui_assets(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 10.0);

            ui.label(page_heading("Asset Packing"));
            ui.label("Convert basic client assets into UODynamapper optimized packages.");
            ui.add_space(6.0);
            draw_assets_overview(ui, self);
            ui.add_space(2.0);

            let is_busy = self.is_busy();
            if is_busy {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
            }

            draw_section_header(ui, "Shared Metadata", "Classic tiledata with Enhanced tileart and string dictionary data");
            draw_asset_cards(ui, "shared_metadata_cards", |cards| {
                cards.card(|ui| self.ui_tilemeta(ui, is_busy));
            });

            ui.add_space(4.0);
            draw_section_header(ui, "Hues and Lights", "Read from both clients: hues.uddp and world_lights.uddp");
            draw_asset_cards(ui, "hues_and_lights_cards", |cards| {
                cards.card(|ui| self.ui_hues(ui, is_busy));
                cards.card(|ui| self.ui_world_lights(ui, is_busy));
            });

            ui.add_space(4.0);
            draw_section_header(ui, "Classic Client", "Assets read from the Classic Client directory");
            draw_asset_cards(ui, "classic_client_cards", |cards| {
                cards.card(|ui| self.ui_tex_art_cc(ui, is_busy));
                cards.card(|ui| self.ui_tex_land_cc(ui, is_busy));
                cards.card(|ui| self.ui_mobile_anim_cc(ui, is_busy));
            });

            ui.add_space(4.0);
            draw_section_header(ui, "Enhanced Client", "Assets read from the Enhanced Client directory");
            draw_asset_cards(ui, "enhanced_client_cards", |cards| {
                cards.card(|ui| self.ui_tex_art_ec(ui, is_busy));
                cards.card(|ui| self.ui_tex_land_ec(ui, is_busy));
                cards.card(|ui| self.ui_tex_aux_ec(ui, is_busy));
                cards.card(|ui| self.ui_mobile_anim_ec(ui, is_busy));
                cards.card(|ui| self.ui_effects_ec(ui, is_busy));
                cards.card(|ui| self.ui_paperdoll_ec(ui, is_busy));
            });

            ui.add_space(4.0);
            draw_section_header(ui, "Gumps", "Interface gumps of both clients: gumps_cc.uddp and gumps_ec.uddp");
            self.ui_gumps(ui, is_busy);
        });
    }

    fn ui_tilemeta(&mut self, ui: &mut egui::Ui, is_busy: bool) {
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
            None,
            vec![],
        );
        if clicked {
            self.convert_tilemeta();
        }
        if stop_clicked { self.request_cancel_conversion(); }
    }

    fn ui_tex_art_cc(&mut self, ui: &mut egui::Ui, is_busy: bool) {
        let tex_art_cc_metadata_source = tex_art_cc_metadata_source_label(self);
        let tex_art_cc_progress = self.asset_pack_progress(AssetPackTask::TexArtCc);
        let (clicked, stop_clicked, preview) = draw_asset_card(
            ui,
            "Classic Art",
            "Items and land textures from art.mul",
            Some(&tex_art_cc_metadata_source),
            &tex_art_cc_progress,
            !is_busy,
            Some(&mut self.settings.zstd_tex_art_cc),
            None,
            Some(&mut self.settings.opt_tex_art_cc),
            None,
            Some((
                &mut self.settings.bc7_rdo_enabled,
                &mut self.settings.bc7_rdo_lambda,
                &mut self.settings.bc7_rdo_lookback_blocks,
            )),
            Some((&mut self.settings.upscale_tex_art_cc, Some(crate::models::UpscalePreviewTarget::TexArtCc))),
            vec![],
        );
        if clicked { self.convert_tex_art_cc(); }
        if stop_clicked { self.request_cancel_conversion(); }
        if let Some((target, filter)) = preview { self.open_upscale_preview(target, filter); }
    }

    fn ui_tex_land_cc(&mut self, ui: &mut egui::Ui, is_busy: bool) {
        let tex_land_cc_progress = self.asset_pack_progress(AssetPackTask::TexLandCc);
        let (clicked, stop_clicked, preview) = draw_asset_card(
            ui,
            "Classic Texmaps",
            "High-resolution terrain textures from texmaps.mul",
            None,
            &tex_land_cc_progress,
            !is_busy,
            Some(&mut self.settings.zstd_tex_land_cc),
            None,
            Some(&mut self.settings.opt_tex_land_cc),
            Some(CLASSIC_LAND_LOSSY_REFUSAL),
            None,
            None,
            vec![
                ("64x64", &mut self.settings.upscale_tex_land_cc_64, crate::models::UpscalePreviewTarget::TexLandCc64),
                ("128x128", &mut self.settings.upscale_tex_land_cc_128, crate::models::UpscalePreviewTarget::TexLandCc128),
            ],
        );
        if clicked { self.convert_tex_land_cc(); }
        if stop_clicked { self.request_cancel_conversion(); }
        if let Some((target, filter)) = preview { self.open_upscale_preview(target, filter); }
    }

    fn ui_mobile_anim_cc(&mut self, ui: &mut egui::Ui, is_busy: bool) {
        let mobile_anim_cc_progress = self.asset_pack_progress(AssetPackTask::MobileAnimCc);
        let (clicked, stop_clicked, _) = draw_asset_card(
            ui,
            "Classic Mobile Animations",
            "Creatures and mobiles from anim*.mul and AnimationFrame*.uop",
            None,
            &mobile_anim_cc_progress,
            !is_busy,
            Some(&mut self.settings.zstd_mobile_anim_cc),
            None,
            Some(&mut self.settings.opt_mobile_anim_cc),
            None,
            Some((
                &mut self.settings.bc7_rdo_enabled,
                &mut self.settings.bc7_rdo_lambda,
                &mut self.settings.bc7_rdo_lookback_blocks,
            )),
            Some((&mut self.settings.upscale_mobile_anim_cc, None)),
            vec![],
        );
        if clicked { self.convert_mobile_anim_cc(); }
        if stop_clicked { self.request_cancel_conversion(); }
    }

    fn ui_tex_art_ec(&mut self, ui: &mut egui::Ui, is_busy: bool) {
        let tex_art_ec_progress = self.asset_pack_progress(AssetPackTask::TexArtEc);
        let (clicked, stop_clicked, preview) = draw_asset_card(
            ui,
            "Enhanced Art",
            "Static items from worldart",
            None,
            &tex_art_ec_progress,
            !is_busy,
            Some(&mut self.settings.zstd_tex_art_ec),
            None,
            Some(&mut self.settings.opt_tex_art_ec),
            None,
            Some((
                &mut self.settings.bc7_rdo_enabled,
                &mut self.settings.bc7_rdo_lambda,
                &mut self.settings.bc7_rdo_lookback_blocks,
            )),
            Some((&mut self.settings.upscale_tex_art_ec, Some(crate::models::UpscalePreviewTarget::TexArtEc))),
            vec![],
        );
        if clicked { self.convert_tex_art_ec(); }
        if stop_clicked { self.request_cancel_conversion(); }
        if let Some((target, filter)) = preview { self.open_upscale_preview(target, filter); }
    }

    fn ui_tex_land_ec(&mut self, ui: &mut egui::Ui, is_busy: bool) {
        let tex_land_ec_progress = self.asset_pack_progress(AssetPackTask::TexLandEc);
        let (clicked, stop_clicked, preview) = draw_asset_card(
            ui,
            "Enhanced Land",
            "High-resolution terrain textures",
            None,
            &tex_land_ec_progress,
            !is_busy,
            Some(&mut self.settings.zstd_tex_land_ec),
            None,
            Some(&mut self.settings.opt_tex_land_ec),
            None,
            // The lossy Enhanced land is BC7 without RDO.
            None,
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
    }

    fn ui_tex_aux_ec(&mut self, ui: &mut egui::Ui, is_busy: bool) {
        let tex_aux_ec_progress = self.asset_pack_progress(AssetPackTask::TexAuxEc);
        let (clicked, stop_clicked, _) = draw_asset_card(
            ui,
            "Enhanced Auxiliary Textures",
            "Non-base auxiliary texture refs (support, alpha mask, noise) from tilemeta",
            None,
            &tex_aux_ec_progress,
            !is_busy,
            Some(&mut self.settings.zstd_tex_aux_ec),
            None,
            Some(&mut self.settings.opt_tex_aux_ec),
            None,
            Some((
                &mut self.settings.bc7_rdo_enabled,
                &mut self.settings.bc7_rdo_lambda,
                &mut self.settings.bc7_rdo_lookback_blocks,
            )),
            Some((&mut self.settings.upscale_tex_aux_ec, None)),
            vec![],
        );
        if clicked { self.convert_tex_aux_ec(); }
        if stop_clicked { self.request_cancel_conversion(); }
    }

    fn ui_mobile_anim_ec(&mut self, ui: &mut egui::Ui, is_busy: bool) {
        let mobile_anim_ec_progress = self.asset_pack_progress(AssetPackTask::MobileAnimEc);
        let (clicked, stop_clicked, _) = draw_asset_card(
            ui,
            "Enhanced Mobile Animations",
            "Creatures and mobiles from AnimationFrame*.uop",
            None,
            &mobile_anim_ec_progress,
            !is_busy,
            Some(&mut self.settings.zstd_mobile_anim_ec),
            None,
            Some(&mut self.settings.opt_mobile_anim_ec),
            None,
            Some((
                &mut self.settings.bc7_rdo_enabled,
                &mut self.settings.bc7_rdo_lambda,
                &mut self.settings.bc7_rdo_lookback_blocks,
            )),
            Some((&mut self.settings.upscale_mobile_anim_ec, None)),
            vec![],
        );
        if clicked { self.convert_mobile_anim_ec(); }
        if stop_clicked { self.request_cancel_conversion(); }
    }

    fn ui_hues(&mut self, ui: &mut egui::Ui, is_busy: bool) {
        let source = hues_source(&self.settings);
        let progress = self.asset_pack_progress(AssetPackTask::Hues);
        let mut clicked = false;
        let mut stop_clicked = false;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
            (clicked, stop_clicked) = draw_asset_card_header(
                ui,
                "Hues",
                "Hue palettes from hues.mul or Hues.uop",
                Some(&hues_source_label(&source)),
                &progress,
                !is_busy,
            );
            ui.separator();
            let settings = &mut self.settings;
            ui.add_enabled_ui(!is_busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(12.0, 8.0);
                    // The ramp spreads hues.mul colours; a Hues.uop is copied as it is.
                    let ramp_applies = !matches!(source, HuesSource::Uop(_));
                    draw_control_cell(ui, "Hue Ramp", HUE_RAMP_CELL_WIDTH, |ui| {
                        ui.horizontal(|ui| {
                            for (ramp, label, tooltip) in [
                                (HueRamp::Linear, "Linear", HUE_RAMP_LINEAR_TOOLTIP),
                                (HueRamp::Flat, "Flat", HUE_RAMP_FLAT_TOOLTIP),
                            ] {
                                if ui
                                    .add_enabled(
                                        ramp_applies,
                                        egui::Button::selectable(settings.hues_ramp == ramp, label),
                                    )
                                    .on_hover_text(tooltip)
                                    .on_disabled_hover_text(HUE_RAMP_REFUSAL)
                                    .clicked()
                                {
                                    settings.hues_ramp = ramp;
                                }
                            }
                        });
                    });
                    draw_package_compression_cells(ui, &mut settings.hues_no_compression, &mut settings.zstd_hues);
                });
            });
        });
        if clicked { self.convert_hues(); }
        if stop_clicked { self.request_cancel_conversion(); }
    }

    fn ui_world_lights(&mut self, ui: &mut egui::Ui, is_busy: bool) {
        let source = world_lights_source_label(&self.settings);
        let progress = self.asset_pack_progress(AssetPackTask::WorldLights);
        let mut clicked = false;
        let mut stop_clicked = false;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
            (clicked, stop_clicked) = draw_asset_card_header(
                ui,
                "World Lights",
                "Light textures from light.mul and TerrainTexture.uop",
                Some(&source),
                &progress,
                !is_busy,
            );
            ui.separator();
            let settings = &mut self.settings;
            ui.add_enabled_ui(!is_busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(12.0, 8.0);
                    draw_package_compression_cells(
                        ui,
                        &mut settings.lights_no_compression,
                        &mut settings.zstd_lights,
                    );
                });
            });
            draw_gump_note(ui, WORLD_LIGHTS_PATCH_NOTE, egui::Color32::GRAY);
        });
        if clicked { self.convert_world_lights(); }
        if stop_clicked { self.request_cancel_conversion(); }
    }

    fn ui_effects_ec(&mut self, ui: &mut egui::Ui, is_busy: bool) {
        let source = effects_ec_source_label(&self.settings);
        let progress = self.asset_pack_progress(AssetPackTask::EffectsEc);
        let mut clicked = false;
        let mut stop_clicked = false;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
            (clicked, stop_clicked) = draw_asset_card_header(
                ui,
                "Enhanced Effects",
                "Effect definitions, particle systems and their textures",
                Some(&source),
                &progress,
                !is_busy,
            );
            ui.separator();
            let settings = &mut self.settings;
            ui.add_enabled_ui(!is_busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(12.0, 8.0);
                    draw_control_cell(ui, "Textures", EFFECTS_FORMAT_CELL_WIDTH, |ui| {
                        ui.horizontal(|ui| {
                            for (bc7, label, tooltip) in [
                                (false, "RGBA8", EFFECTS_EC_RGBA_TOOLTIP),
                                (true, "BC7", EFFECTS_EC_BC7_TOOLTIP),
                            ] {
                                if ui
                                    .add(egui::Button::selectable(settings.effects_ec_bc7 == bc7, label))
                                    .on_hover_text(tooltip)
                                    .clicked()
                                {
                                    settings.effects_ec_bc7 = bc7;
                                }
                            }
                        });
                    });
                    draw_package_zstd_cell(ui, &mut settings.zstd_effects_ec, true, EFFECTS_EC_ZSTD_TOOLTIP);
                    draw_control_cell(ui, "Error Findings", EFFECTS_STRICT_CELL_WIDTH, |ui| {
                        ui.checkbox(&mut settings.effects_ec_strict, "Strict")
                            .on_hover_text(EFFECTS_EC_STRICT_TOOLTIP);
                    });
                    draw_findings_report_cell(
                        ui,
                        &mut settings.effects_ec_report,
                        EFFECTS_EC_REPORT_DEFAULT_NAME,
                        EFFECTS_EC_REPORT_TOOLTIP,
                        "None: the summary counts the findings",
                    );
                });
            });
        });
        if clicked { self.convert_effects_ec(); }
        if stop_clicked { self.request_cancel_conversion(); }
    }

    fn ui_paperdoll_ec(&mut self, ui: &mut egui::Ui, is_busy: bool) {
        let source = paperdoll_ec_source_label(&self.settings);
        let progress = self.asset_pack_progress(AssetPackTask::PaperdollEc);
        let mut clicked = false;
        let mut stop_clicked = false;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
            (clicked, stop_clicked) = draw_asset_card_header(
                ui,
                "Enhanced Paperdoll",
                "3D paperdoll frames, lossless, with the hue tables they take and what each robe takes off",
                Some(&source),
                &progress,
                !is_busy,
            );
            ui.separator();
            let settings = &mut self.settings;
            ui.add_enabled_ui(!is_busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(12.0, 8.0);
                    draw_package_zstd_cell(ui, &mut settings.zstd_paperdoll_ec, true, PAPERDOLL_EC_ZSTD_TOOLTIP);
                    draw_findings_report_cell(
                        ui,
                        &mut settings.paperdoll_ec_report,
                        PAPERDOLL_EC_REPORT_DEFAULT_NAME,
                        PAPERDOLL_EC_REPORT_TOOLTIP,
                        "None: the log lists each decision",
                    );
                });
            });
        });
        if clicked { self.convert_paperdoll_ec(); }
        if stop_clicked { self.request_cancel_conversion(); }
    }

    fn ui_gumps(&mut self, ui: &mut egui::Ui, is_busy: bool) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!is_busy, egui::Button::new(egui::RichText::new("Pack Classic + Enhanced").strong()))
                .on_hover_text(GUMPS_COMBINED_TOOLTIP)
                .on_disabled_hover_text("Another task is running")
                .clicked()
            {
                self.convert_gumps_cc_and_ec();
            }
            ui.label(egui::RichText::new("Classic first, so the Enhanced gumps can share with it").weak());
        });
        draw_asset_cards(ui, "gumps_cards", |cards| {
            cards.card(|ui| self.ui_gumps_cc(ui, is_busy));
            cards.card(|ui| self.ui_gumps_ec(ui, is_busy));
        });
    }

    fn ui_gumps_cc(&mut self, ui: &mut egui::Ui, is_busy: bool) {
        let gumps_cc_source = gumps_cc_source_label(self);
        let gumps_cc_progress = self.asset_pack_progress(AssetPackTask::GumpsCc);
        let mut clicked = false;
        let mut stop_clicked = false;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
            (clicked, stop_clicked) = draw_asset_card_header(
                ui,
                "Classic Gumps",
                "Gumps from gumpart.mul/gumpidx.mul or gumpartLegacyMUL.uop",
                Some(&gumps_cc_source),
                &gumps_cc_progress,
                !is_busy,
            );
            ui.separator();
            let settings = &mut self.settings;
            ui.add_enabled_ui(!is_busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(12.0, 8.0);
                    draw_control_cell(ui, "Read First", GUMP_SOURCE_CELL_WIDTH, |ui| {
                        ui.horizontal(|ui| {
                            for preference in [SourceFormatPreference::Mul, SourceFormatPreference::Uop] {
                                let label = preference.preferred_first()[0].label();
                                if ui
                                    .add(egui::Button::selectable(settings.gumps_cc_source == preference, label))
                                    .on_hover_text(GUMPS_CC_SOURCE_TOOLTIP)
                                    .clicked()
                                {
                                    settings.gumps_cc_source = preference;
                                }
                            }
                        });
                    });
                    draw_gump_format_cell(
                        ui,
                        &mut settings.gumps_cc_bc1,
                        CC_LOSSY_SINGLE_GUMP_FORMAT,
                        GUMPS_CC_BC1_TOOLTIP,
                        EC_LOSSY_SINGLE_GUMP_FORMAT,
                        GUMPS_CC_BC7_REFUSAL,
                    );
                    draw_gump_zstd_cell(ui, &mut settings.zstd_gumps_cc);
                    draw_gump_upscale_cell(
                        ui,
                        "Paperdoll Upscale",
                        "gumps_cc_paperdoll_upscale",
                        &mut settings.upscale_gumps_cc_paperdoll,
                        GUMPS_PAPERDOLL_UPSCALE_TOOLTIP,
                    );
                    draw_gump_upscale_cell(
                        ui,
                        "Single Gump Upscale",
                        "gumps_cc_single_upscale",
                        &mut settings.upscale_gumps_cc_single,
                        GUMPS_SINGLE_UPSCALE_TOOLTIP,
                    );
                });
            });
            draw_gump_note(ui, &gumps_lossless_note("Classic partial hue needs"), egui::Color32::GRAY);
        });
        if clicked { self.convert_gumps_cc(); }
        if stop_clicked { self.request_cancel_conversion(); }
    }

    fn ui_gumps_ec(&mut self, ui: &mut egui::Ui, is_busy: bool) {
        let ec_source = EcGumpSource::of(&self.settings);
        let gumps_cc = self.gumps_cc_output();
        let share = ec_gumps_share(self.settings.gumps_ec_sharing, &gumps_cc, gumps_cc.is_file());
        let gumps_ec_progress = self.asset_pack_progress(AssetPackTask::GumpsEc);
        let mut clicked = false;
        let mut stop_clicked = false;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);
            (clicked, stop_clicked) = draw_asset_card_header(
                ui,
                "Enhanced Gumps",
                "Gumps from Interface.uop or an extracted gumpart folder",
                Some(&ec_source.label()),
                &gumps_ec_progress,
                !is_busy,
            );
            ui.separator();
            let settings = &mut self.settings;
            ui.add_enabled_ui(!is_busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(12.0, 8.0);
                    draw_control_cell(ui, "Extracted Gumpart Folder", GUMP_GUMPART_CELL_WIDTH, |ui| {
                        ui.horizontal(|ui| {
                            if ui.button("Select...").on_hover_text(GUMPS_EC_GUMPART_TOOLTIP).clicked() {
                                if let Some(dir) = uot_gui::pick_folder() {
                                    settings.gumps_ec_gumpart_dir = Some(dir);
                                }
                            }
                            if ui
                                .add_enabled(settings.gumps_ec_gumpart_dir.is_some(), egui::Button::new("Clear"))
                                .on_hover_text("Read the Enhanced client folder")
                                .clicked()
                            {
                                settings.gumps_ec_gumpart_dir = None;
                            }
                        });
                        let folder = match &settings.gumps_ec_gumpart_dir {
                            Some(dir) => dir.display().to_string(),
                            None => "None: the Enhanced client folder".to_string(),
                        };
                        ui.label(egui::RichText::new(folder).weak());
                    });
                    draw_control_cell(ui, "Max Gump Id", GUMP_MAX_ID_CELL_WIDTH, |ui| {
                        ui.add_enabled(
                            !matches!(ec_source, EcGumpSource::Extracted(_)),
                            egui::DragValue::new(&mut settings.gumps_ec_max_id).speed(GUMP_MAX_ID_DRAG_SPEED),
                        )
                        .on_hover_text(GUMPS_EC_MAX_ID_TOOLTIP)
                        .on_disabled_hover_text(GUMPS_EC_MAX_ID_REFUSAL);
                    });
                    draw_gump_format_cell(
                        ui,
                        &mut settings.gumps_ec_bc7,
                        EC_LOSSY_SINGLE_GUMP_FORMAT,
                        GUMPS_EC_BC7_TOOLTIP,
                        CC_LOSSY_SINGLE_GUMP_FORMAT,
                        GUMPS_EC_BC1_REFUSAL,
                    );
                    draw_gump_zstd_cell(ui, &mut settings.zstd_gumps_ec);
                    draw_gump_upscale_cell(
                        ui,
                        "Paperdoll Upscale",
                        "gumps_ec_paperdoll_upscale",
                        &mut settings.upscale_gumps_ec_paperdoll,
                        GUMPS_PAPERDOLL_UPSCALE_TOOLTIP,
                    );
                    draw_gump_upscale_cell(
                        ui,
                        "Single Gump Upscale",
                        "gumps_ec_single_upscale",
                        &mut settings.upscale_gumps_ec_single,
                        GUMPS_SINGLE_UPSCALE_TOOLTIP,
                    );
                    let share_tooltip = gumps_ec_share_tooltip(settings);
                    draw_control_cell(ui, "Classic Twins", GUMP_SHARING_CELL_WIDTH, |ui| {
                        ui.horizontal(|ui| {
                            for (sharing, label) in [
                                (GumpsEcSharing::ShareWithGumpsCc, "Share with gumps_cc"),
                                (GumpsEcSharing::Standalone, "Pack standalone"),
                            ] {
                                if ui
                                    .add(egui::Button::selectable(settings.gumps_ec_sharing == sharing, label))
                                    .on_hover_text(share_tooltip.as_str())
                                    .clicked()
                                {
                                    settings.gumps_ec_sharing = sharing;
                                }
                            }
                        });
                    });
                });
            });
            let (share_note, share_colour) = gumps_ec_share_note(&share);
            draw_gump_note(ui, &share_note, share_colour);
            if self.settings.gumps_ec_bc7 && !self.settings.upscale_gumps_ec_single.keeps_palette() {
                draw_gump_note(
                    ui,
                    &format!(
                        "{:?} blends colours, so the upscaled gumps leave the Classic colour space: BC7, not BC1.",
                        self.settings.upscale_gumps_ec_single
                    ),
                    NOTICE_COLOUR,
                );
            }
            draw_gump_note(ui, &gumps_lossless_note("the Enhanced client hues"), egui::Color32::GRAY);
        });
        if clicked { self.convert_gumps_ec(); }
        if stop_clicked { self.request_cancel_conversion(); }
    }
}

const GUMP_SOURCE_CELL_WIDTH: f32 = 100.0;
const GUMP_FORMAT_CELL_WIDTH: f32 = 190.0;
const GUMP_ZSTD_CELL_WIDTH: f32 = 140.0;
const GUMP_UPSCALE_CELL_WIDTH: f32 = 190.0;
const GUMP_GUMPART_CELL_WIDTH: f32 = 250.0;
const GUMP_MAX_ID_CELL_WIDTH: f32 = 96.0;
const GUMP_SHARING_CELL_WIDTH: f32 = 250.0;
const GUMP_MAX_ID_DRAG_SPEED: f64 = 10.0;
/// The Zstd levels a level field accepts.
const ZSTD_LEVEL_RANGE: std::ops::RangeInclusive<i32> = 1..=22;
/// The colour of a note that a choice changes what the job writes.
const NOTICE_COLOUR: egui::Color32 = egui::Color32::from_rgb(255, 190, 110);
/// The colour of a note on where the job reads or shares from.
const SOURCE_NOTE_COLOUR: egui::Color32 = egui::Color32::from_rgb(100, 200, 255);

const GUMPS_COMBINED_TOOLTIP: &str = "Packs gumps_cc.uddp, then gumps_ec.uddp, which shares with the new gumps_cc when \
Share with gumps_cc is chosen.";
const GUMPS_RGBA_TOOLTIP: &str = "Lossless RGBA8888: the default.";
const GUMPS_CC_SOURCE_TOOLTIP: &str = "The Classic gump files read first: gumpart.mul/gumpidx.mul (MUL) or \
gumpartLegacyMUL.uop (UOP, pack-gumps --uop). The other one is the fallback. verdata.mul patches the MUL files only.";
const GUMPS_CC_BC1_TOOLTIP: &str = "Every single (non-paperdoll) Classic gump as BC1 (pack-gumps --bc1): lossy, with \
one bit of alpha, and Zstd 19 unless a level is set.";
const GUMPS_CC_BC7_REFUSAL: &str = "gumps_cc packs lossy as BC1 only (pack-gumps --bc1): Classic gumps have one bit \
of alpha, which BC1 holds in half the size of BC7.";
const GUMPS_EC_BC7_TOOLTIP: &str = "The single (non-paperdoll) Enhanced gumps lossy (pack-ec-gumps --bc7), with Zstd 19 \
unless a level is set: the gumps in the Classic colour space (alpha 0 or 255, colours on Classic's steps of 8, as the \
Classic gumps Interface.uop repeats) as BC1, the others as BC7, soft alpha kept. The test runs after any upscale pass, \
the upscale profile's too, so an interpolating upscaler sends a gump to BC7.";
const GUMPS_EC_BC1_REFUSAL: &str = "Enhanced gumps pack lossy as BC7, which keeps soft alpha; with BC7 chosen, the \
gumps in the Classic colour space become BC1 by themselves.";
const GUMPS_ZSTD_TOOLTIP: &str = "Zstd level of the single gumps and the paperdoll atlas pages (--zstd=LEVEL). By \
default the single gumps use 7 (19 when lossy) and the atlas pages, lossless, 19.";
const GUMPS_PAPERDOLL_UPSCALE_TOOLTIP: &str = "Upscales the paperdoll equipment gumps on their atlas pages \
(--paperdoll-upscale-pass). An upscale profile can set other passes per image type and gump id.";
const GUMPS_SINGLE_UPSCALE_TOOLTIP: &str = "Upscales the other gumps (--single-upscale-pass). An upscale profile can \
set other passes per image type and gump id.";
const GUMPS_EC_GUMPART_TOOLTIP: &str = "An extracted gumpart folder to read instead of the Enhanced client's \
Interface.uop: a folder holding data/interface/default/textures/gumpart, or a folder of Enhanced gump images.";
const GUMPS_EC_MAX_ID_TOOLTIP: &str = "The highest gump id read from Interface.uop (pack-ec-gumps --max-id).";
const GUMPS_EC_MAX_ID_REFUSAL: &str = "An extracted gumpart folder is read whole; only Interface.uop is read by id.";
const GUMPS_EC_SHARE_TOOLTIP: &str = "Share with gumps_cc: leave out each single gump the output folder's gumps_cc.uddp \
draws the same (the Classic gumps Interface.uop repeats) and list it for Dynamapper to draw from the Classic source \
(pack-ec-gumps --share-with). The package then needs that gumps_cc.uddp, or the Classic client files, to show those \
gumps. With upscaling, the Classic and Enhanced single gump passes must match, or no gump draws the same and nothing is \
shared.\n\nPack standalone: keep every gump, so the package needs no gumps_cc.";

/// The width of an asset card's progress bar.
const CARD_PROGRESS_BAR_WIDTH: f32 = 200.0;
/// The size of a running job's Stop button, and so the height of a card's title row.
const STOP_BUTTON_SIZE: [f32; 2] = [90.0, 30.0];
const HUE_RAMP_CELL_WIDTH: f32 = 110.0;
const PACKAGE_COMPRESSION_CELL_WIDTH: f32 = 110.0;
const PACKAGE_ZSTD_CELL_WIDTH: f32 = 140.0;
const EFFECTS_FORMAT_CELL_WIDTH: f32 = 110.0;
const EFFECTS_STRICT_CELL_WIDTH: f32 = 100.0;
const FINDINGS_REPORT_CELL_WIDTH: f32 = 250.0;
/// The file names the findings report dialogs offer.
const EFFECTS_EC_REPORT_DEFAULT_NAME: &str = "effects_ec_findings.txt";
const PAPERDOLL_EC_REPORT_DEFAULT_NAME: &str = "paperdoll_ec_findings.txt";

const HUE_RAMP_LINEAR_TOOLTIP: &str = "Each hues.mul colour at every 8th texel, ramping linearly to the next, as EA's \
Hues.uop does (pack-hues --hue-ramp linear): the default.";
const HUE_RAMP_FLAT_TOOLTIP: &str = "Each hues.mul colour repeated over 8 texels (pack-hues --hue-ramp flat).";
const HUE_RAMP_REFUSAL: &str = "No folder holds a hues.mul, so the Enhanced Hues.uop is read and copied as it is: the \
ramp applies only to hues.mul colours.";
const PACKAGE_RAW_TOOLTIP: &str = "Store the package's entries raw, without Zstd (--no-compression).";
const PACKAGE_ZSTD_TOOLTIP: &str = "Zstd level of the package's entries (--zstd=LEVEL). Default: the packer's level, 7.";
const PACKAGE_ZSTD_REFUSAL: &str = "The package is stored raw: nothing is compressed.";
const WORLD_LIGHTS_PATCH_NOTE: &str = "verdata.mul (Classic patch files, above) patches the Classic lights; the dif \
options don't apply to them.";
const EFFECTS_EC_RGBA_TOOLTIP: &str = "Uncompressed source textures stay RGBA8: the default. DXT1/DXT5 sources stay \
BC1/BC3.";
const EFFECTS_EC_BC7_TOOLTIP: &str = "Encode uncompressed textures whose size is a multiple of 4 to BC7 (pack-ec-effects \
--bc7); the others stay RGBA8. DXT1/DXT5 sources stay BC1/BC3.";
const EFFECTS_EC_ZSTD_TOOLTIP: &str = "Zstd level of the manifests and textures (pack-ec-effects --zstd=LEVEL). Default: \
the packer's level, 7. The particle system blobs always use the package's type dictionary.";
const EFFECTS_EC_STRICT_TOOLTIP: &str = "Fail on error findings instead of writing flagged records (pack-ec-effects \
--strict); particle files missing from the client are warnings.";
const EFFECTS_EC_REPORT_TOOLTIP: &str = "Write every finding of the packer to this file, one line each \
(pack-ec-effects --report), before the strict check.";
const PAPERDOLL_EC_ZSTD_TOOLTIP: &str = "Zstd level of the frames and manifests (pack-ec-paperdoll --zstd=LEVEL). \
Default: the packer's level, 19. The frames are lossless at every level.";
const PAPERDOLL_EC_REPORT_TOOLTIP: &str = "Write every finding of the dress-layer measurement to this file, one line \
each (pack-ec-paperdoll --report): the largest garment of each slot and body, each robe's decision and the texels each \
garment drawn beneath it loses. The log lists the decisions either way.";

fn hues_source_label(source: &HuesSource) -> String {
    match source {
        HuesSource::NoFolder => "Source: select a Classic or Enhanced source directory".to_string(),
        HuesSource::Mul(path) => format!("Source (MUL): {}", path.display()),
        HuesSource::Uop(path) => format!("Source (UOP, copied as it is): {}", path.display()),
        HuesSource::Missing => "Source: missing hues.mul or Hues.uop".to_string(),
    }
}

fn world_lights_source_label(settings: &AppSettings) -> String {
    let folders: Vec<String> = [
        settings.cc_dir.as_ref().map(|dir| format!("light.mul from {}", dir.display())),
        settings.ec_dir.as_ref().map(|dir| format!("TerrainTexture.uop from {}", dir.display())),
    ]
    .into_iter()
    .flatten()
    .collect();
    if folders.is_empty() {
        "Source: select a Classic or Enhanced source directory".to_string()
    } else {
        format!("Source: {}", folders.join("; "))
    }
}

fn effects_ec_source_label(settings: &AppSettings) -> String {
    if settings.ec_dir.is_none() {
        return "Source: select an Enhanced source directory".to_string();
    }
    match effects_ec_source(settings) {
        Some(path) => format!("Source: {}, with the Enhanced effect definitions", path.display()),
        None => format!("Source: missing {EFFECT_TEXTURE_UOP_NAME}"),
    }
}

fn paperdoll_ec_source_label(settings: &AppSettings) -> String {
    if settings.ec_dir.is_none() {
        return "Source: select an Enhanced source directory".to_string();
    }
    match paperdoll_ec_sources(settings) {
        [Some(frames), Some(hues), Some(tileart), Some(_)] => {
            format!("Source: {}, {} and {}", frames.display(), hues.display(), tileart.display())
        }
        sources => {
            let missing: Vec<&str> = PAPERDOLL_EC_SOURCE_NAMES
                .iter()
                .zip(&sources)
                .filter(|(_, found)| found.is_none())
                .map(|(&name, _)| name)
                .collect();
            format!("Source: missing {}", missing.join(", "))
        }
    }
}

/// A packer's findings file: chosen with a save dialog, cleared, and shown below; `none` says what happens without it.
fn draw_findings_report_cell(
    ui: &mut egui::Ui,
    report: &mut Option<std::path::PathBuf>,
    default_name: &str,
    tooltip: &str,
    none: &str,
) {
    draw_control_cell(ui, "Findings Report", FINDINGS_REPORT_CELL_WIDTH, |ui| {
        ui.horizontal(|ui| {
            if ui.button("Select...").on_hover_text(tooltip).clicked() {
                if let Some(path) = uot_gui::save_file_dialog(default_name) {
                    *report = Some(path);
                }
            }
            if ui
                .add_enabled(report.is_some(), egui::Button::new("Clear"))
                .on_hover_text("Write no findings file")
                .clicked()
            {
                *report = None;
            }
        });
        let shown = match report {
            Some(path) => path.display().to_string(),
            None => none.to_string(),
        };
        ui.label(egui::RichText::new(shown).weak());
    });
}

/// The package codec as `--no-compression` and `--zstd[=LEVEL]` set it: raw, or Zstd at the packer's or a chosen level.
fn draw_package_compression_cells(ui: &mut egui::Ui, no_compression: &mut bool, zstd_level: &mut Option<i32>) {
    draw_control_cell(ui, "Compression", PACKAGE_COMPRESSION_CELL_WIDTH, |ui| {
        ui.checkbox(no_compression, "Stored raw").on_hover_text(PACKAGE_RAW_TOOLTIP);
    });
    draw_package_zstd_cell(ui, zstd_level, !*no_compression, PACKAGE_ZSTD_TOOLTIP);
}

/// A package's Zstd level: the packer's by default, or a chosen one; greyed out when the package is stored raw.
fn draw_package_zstd_cell(ui: &mut egui::Ui, level: &mut Option<i32>, compressed: bool, tooltip: &str) {
    draw_control_cell(ui, "Zstd Level", PACKAGE_ZSTD_CELL_WIDTH, |ui| {
        ui.horizontal(|ui| {
            let mut packer_level = level.is_none();
            if ui
                .add_enabled(compressed, egui::Checkbox::new(&mut packer_level, "Default"))
                .on_hover_text(tooltip)
                .on_disabled_hover_text(PACKAGE_ZSTD_REFUSAL)
                .changed()
            {
                *level = (!packer_level).then_some(DEFAULT_ZSTD_LEVEL);
            }
            if let Some(level) = level {
                ui.add_enabled(compressed, egui::DragValue::new(level).speed(1).range(ZSTD_LEVEL_RANGE))
                    .on_hover_text(tooltip)
                    .on_disabled_hover_text(PACKAGE_ZSTD_REFUSAL);
            }
        });
    });
}

/// Where the Enhanced gumps job reads, as `convert_ec_gumps_to_uddp_from_sources` picks it.
enum EcGumpSource {
    NoFolder,
    InterfaceUop(std::path::PathBuf),
    Extracted(std::path::PathBuf),
    Missing,
}

impl EcGumpSource {
    fn of(settings: &AppSettings) -> Self {
        let sources = ec_gumps_source_dirs(settings);
        if sources.is_empty() {
            Self::NoFolder
        } else if let Some(interface_uop) = find_interface_uop(&sources) {
            Self::InterfaceUop(interface_uop)
        } else if let Some(gumpart_dir) = find_ec_gumpart_dir(&sources) {
            Self::Extracted(gumpart_dir)
        } else {
            Self::Missing
        }
    }

    fn label(&self) -> String {
        match self {
            Self::NoFolder => "Source: select an Enhanced source directory or a gumpart folder".to_string(),
            Self::InterfaceUop(path) => format!("Source (UOP): {}", path.display()),
            Self::Extracted(path) => format!("Source (extracted gumpart): {}", path.display()),
            Self::Missing => "Source: missing Interface.uop or an extracted gumpart folder".to_string(),
        }
    }
}

fn gumps_cc_source_label(app: &UddConvApp) -> String {
    let sources = cc_gumps_source_dirs(&app.settings);
    if sources.is_empty() {
        return "Source: select a Classic or Enhanced source directory".to_string();
    }
    match resolve_classic_gump_source(&sources, app.settings.gumps_cc_source) {
        Ok(source) => format!("Source ({}): {}", source.format.label(), source.primary_path.display()),
        Err(_) => "Source: missing gumpart.mul/gumpidx.mul or gumpartLegacyMUL.uop".to_string(),
    }
}

/// The share choice's tooltip, which also says when the single gump upscale filters differ.
fn gumps_ec_share_tooltip(settings: &AppSettings) -> String {
    if settings.upscale_gumps_cc_single == settings.upscale_gumps_ec_single {
        GUMPS_EC_SHARE_TOOLTIP.to_string()
    } else {
        format!(
            "{GUMPS_EC_SHARE_TOOLTIP}\n\nThe single gump upscale filters differ now ({:?} for Classic, {:?} for \
             Enhanced), so nothing would be shared.",
            settings.upscale_gumps_cc_single, settings.upscale_gumps_ec_single
        )
    }
}

fn gumps_ec_share_note(share: &EcGumpsShare) -> (String, egui::Color32) {
    match share {
        EcGumpsShare::With(gumps_cc) => (
            format!("Shares the Classic twins with {}", gumps_cc.display()),
            SOURCE_NOTE_COLOUR,
        ),
        EcGumpsShare::MissingGumpsCc(gumps_cc) => (
            format!(
                "No {} yet: Pack Asset packs standalone; Pack Classic + Enhanced builds it first and shares with it.",
                gumps_cc.display()
            ),
            NOTICE_COLOUR,
        ),
        EcGumpsShare::Standalone => (
            "Packs standalone: every gump kept.".to_string(),
            egui::Color32::GRAY,
        ),
    }
}

/// The gumps every format leaves RGBA8888, built from the id ranges the packers use.
fn gumps_lossless_note(hue: &str) -> String {
    let mut body_gumps: Vec<u32> = PAPERDOLL_BODY_GUMP_BY_BODY
        .iter()
        .map(|&(_, body_gump)| body_gump)
        .filter(|&body_gump| !is_paperdoll_equipment_gump_id(body_gump))
        .collect();
    body_gumps.sort_unstable();
    body_gumps.dedup();
    let body_gumps: Vec<String> = body_gumps.iter().map(u32::to_string).collect();
    format!(
        "Always RGBA8888: the paperdoll gumps ({PAPERDOLL_EQUIPMENT_GUMP_ID_START}-{PAPERDOLL_EQUIPMENT_GUMP_ID_END}) on \
         their lossless atlas pages, and the paperdoll body gumps ({}): {hue} exact greys.",
        body_gumps.join(", ")
    )
}

fn draw_gump_note(ui: &mut egui::Ui, text: &str, colour: egui::Color32) {
    ui.label(egui::RichText::new(text).size(12.5).color(colour));
}

/// The single gump formats as toggles: lossless RGBA8888, the package's lossy format, and the other lossy format greyed
/// out with `refusal` as the reason.
fn draw_gump_format_cell(
    ui: &mut egui::Ui,
    lossy: &mut bool,
    lossy_format: GumpPixelFormat,
    lossy_tooltip: &str,
    refused_format: GumpPixelFormat,
    refusal: &str,
) {
    draw_control_cell(ui, "Single Gumps", GUMP_FORMAT_CELL_WIDTH, |ui| {
        ui.horizontal(|ui| {
            let current = single_gump_pixel_format(*lossy, lossy_format);
            for format in [GumpPixelFormat::Rgba8888, GumpPixelFormat::Bc1, GumpPixelFormat::Bc7] {
                let button = egui::Button::selectable(current == format, format.label());
                if format == refused_format {
                    ui.add_enabled(false, button).on_disabled_hover_text(refusal);
                    continue;
                }
                let tooltip = if format == lossy_format { lossy_tooltip } else { GUMPS_RGBA_TOOLTIP };
                if ui.add(button).on_hover_text(tooltip).clicked() {
                    *lossy = format == lossy_format;
                }
            }
        });
    });
}

/// The gump Zstd level: the package's levels by default, or one level for the single gumps and atlas pages.
fn draw_gump_zstd_cell(ui: &mut egui::Ui, level: &mut Option<i32>) {
    draw_control_cell(ui, "Zstd Level", GUMP_ZSTD_CELL_WIDTH, |ui| {
        ui.horizontal(|ui| {
            let mut package_levels = level.is_none();
            if ui.checkbox(&mut package_levels, "Default").on_hover_text(GUMPS_ZSTD_TOOLTIP).changed() {
                *level = (!package_levels).then_some(LOSSLESS_PAGE_ZSTD_LEVEL);
            }
            if let Some(level) = level {
                ui.add(egui::DragValue::new(level).speed(1).range(ZSTD_LEVEL_RANGE))
                    .on_hover_text(GUMPS_ZSTD_TOOLTIP);
            }
        });
    });
}

fn draw_gump_upscale_cell(ui: &mut egui::Ui, label: &str, id: &str, filter: &mut UpscaleFilter, tooltip: &str) {
    draw_control_cell(ui, label, GUMP_UPSCALE_CELL_WIDTH, |ui| {
        ui.scope(|ui| draw_upscale_filter(ui, id.to_string(), filter))
            .response
            .on_hover_text(tooltip);
    });
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
                        egui::RichText::new("Upscale profile")
                            .strong()
                            .color(egui::Color32::from_rgb(150, 220, 170)),
                    );
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Select File...").clicked() {
                            let mut dialog = uot_gui::file_dialog()
                                .set_title("Select Upscale Profile")
                                .add_filter("Upscale profile", &["toml", "kdl"]);
                            if let Some(path) = app
                                .settings
                                .upscale_profile_path
                                .as_ref()
                                .and_then(|path| path.parent())
                            {
                                dialog = dialog.set_directory(path);
                            }
                            if let Some(path) = dialog.pick_file() {
                                app.settings.upscale_profile_path = Some(path);
                                app.persist_settings();
                            }
                        }
                        if ui
                            .add_enabled(
                                app.settings.upscale_profile_path.is_some(),
                                egui::Button::new("Clear"),
                            )
                            .clicked()
                        {
                            app.settings.upscale_profile_path = None;
                            app.persist_settings();
                        }
                    });
                    if let Some(path) = &app.settings.upscale_profile_path {
                        ui.label(egui::RichText::new(path.to_string_lossy()).weak());
                    } else {
                        ui.label(egui::RichText::new("No profile selected").weak());
                    }
                });

                ui.separator();

                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new("Compression")
                            .strong()
                            .color(egui::Color32::from_rgb(100, 200, 255)),
                    );
                    ui.label(egui::RichText::new("BC7 saves VRAM. BC7+zstd saves disk. Raw+zstd is lossless.").weak());
                    ui.label(egui::RichText::new("BC7 outputs use automatic 4x4-aligned atlas placement.").weak());
                    ui.label(egui::RichText::new("Land textures always include filtering-aware gutters.").weak());
                });

                ui.separator();

                ui.vertical(|ui| {
                    ui.label(
                        egui::RichText::new("Validation")
                            .strong()
                            .color(egui::Color32::from_rgb(190, 170, 255)),
                    );
                    ui.add_enabled(
                        !app.is_busy(),
                        egui::Checkbox::new(&mut app.settings.validate_after_packing, "Validate packages after packing"),
                    )
                    .on_hover_text(VALIDATE_AFTER_PACKING_TOOLTIP)
                    .on_disabled_hover_text("Another task is running");
                });
            });
        });
}

fn draw_section_header(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        ui.label(section_heading(title));
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
    texture_zstd_level: Option<&mut i32>,
    zstd_only_level: Option<&mut i32>,
    opt: Option<&mut TextureOptimization>,
    // When set, the block formats are greyed out with this reason in their tooltip.
    lossy_refusal: Option<&str>,
    bc7_rdo: Option<(&mut bool, &mut f32, &mut usize)>,
    upscale_single: Option<(&mut UpscaleFilter, Option<crate::models::UpscalePreviewTarget>)>,
    mut upscale_configs: Vec<(&str, &mut udd_conv::upscale::UpscaleConfig, crate::models::UpscalePreviewTarget)>,
) -> (bool, bool, Option<(crate::models::UpscalePreviewTarget, UpscaleFilter)>) {
    let mut clicked = false;
    let mut stop_clicked = false;
    let mut preview_req = None;

    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 8.0);

        (clicked, stop_clicked) = draw_asset_card_header(ui, title, desc, source_note, progress, can_start);

        if opt.is_some()
            || texture_zstd_level.is_some()
            || zstd_only_level.is_some()
            || bc7_rdo.is_some()
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
                        let show_bc7_settings = matches!(opt_val, TextureOptimization::Bc7Zstd);
                        draw_control_cell(ui, "Output Format", 210.0, |ui| {
                            let current_fmt = match opt_val {
                                TextureOptimization::None => "Raw+zstd",
                                TextureOptimization::Bc7 => "BC7",
                                TextureOptimization::Bc7Zstd => "Supercompressed BC7+zstd",
                            };

                            egui::ComboBox::from_id_salt(format!("{}_fmt", title))
                                .selected_text(current_fmt)
                                .width(ui.available_width())
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(
                                            current_fmt == "Raw+zstd",
                                            "Raw+zstd",
                                        )
                                        .clicked()
                                    {
                                        *opt_val = TextureOptimization::None;
                                    }
                                    if ui
                                        .add_enabled(
                                            lossy_refusal.is_none(),
                                            egui::Button::selectable(current_fmt == "BC7", "BC7"),
                                        )
                                        .on_disabled_hover_text(lossy_refusal.unwrap_or_default())
                                        .clicked()
                                    {
                                        *opt_val = TextureOptimization::Bc7;
                                    }
                                    if ui
                                        .add_enabled(
                                            lossy_refusal.is_none(),
                                            egui::Button::selectable(
                                                current_fmt == "Supercompressed BC7+zstd",
                                                "Supercompressed BC7+zstd",
                                            ),
                                        )
                                        .on_disabled_hover_text(lossy_refusal.unwrap_or_default())
                                        .clicked()
                                    {
                                        *opt_val = TextureOptimization::Bc7Zstd;
                                    }
                                });
                        });

                        if show_bc7_settings {
                            if let Some((enabled, lambda, lookback_blocks)) = bc7_rdo {
                                *lookback_blocks =
                                    udd_conv::bc7::normalize_bc7_rdo_lookback_blocks(
                                        *lookback_blocks,
                                    );
                                draw_control_cell(ui, "BC7 RDO", 230.0, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.checkbox(enabled, "On")
                                            .on_hover_text("Disable to skip the BC7 RDO pass.");
                                        ui.add_enabled(
                                            *enabled,
                                            egui::DragValue::new(lambda)
                                                .speed(0.01)
                                                .range(0.0..=1.0),
                                        )
                                            .on_hover_text("Higher lambda values usually improve BC7+zstd disk ratio but can be much slower and may add more loss.");
                                        ui.add_enabled_ui(*enabled, |ui| {
                                            egui::ComboBox::from_id_salt(format!("{title}_bc7_rdo_lookback"))
                                                .selected_text(format!("{} blocks", *lookback_blocks))
                                                .width(82.0)
                                                .show_ui(ui, |ui| {
                                                    for blocks in udd_conv::bc7::BC7_RDO_LOOKBACK_BLOCK_PRESETS {
                                                        ui.selectable_value(
                                                            lookback_blocks,
                                                            blocks,
                                                            format!("{blocks} blocks"),
                                                        );
                                                    }
                                                });
                                        })
                                            .response
                                            .on_hover_text("Limits how many previous BC7 blocks RDO searches.");
                                    });
                                });
                            }
                        }

                        if let Some(zstd_level) = texture_zstd_level {
                            match *opt_val {
                                TextureOptimization::None | TextureOptimization::Bc7Zstd => {
                                    draw_zstd_level_cell(ui, zstd_level);
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
                                let combo_width = if target.is_some() {
                                    (ui.available_width() - 34.0).max(120.0)
                                } else {
                                    ui.available_width().max(120.0)
                                };
                                ui.scope(|ui| {
                                    ui.set_width(combo_width);
                                    draw_upscale_filter(ui, format!("{}_upscale", title), up_val);
                                });
                                if let Some(target) = target {
                                    if ui.button("🔍").on_hover_text("Preview").clicked() {
                                        preview_req = Some((target, *up_val));
                                    }
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

    (clicked, stop_clicked, preview_req)
}

/// The frame of an asset card.
fn asset_card_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::group(ui.style())
        .fill(ui.visuals().widgets.noninteractive.bg_fill)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::same(10))
}

/// Lays out a section's asset cards, each in an asset card frame: two per row when the width allows.
fn draw_asset_cards(ui: &mut egui::Ui, id_salt: &str, add_cards: impl FnOnce(&mut CardGrid<'_>)) {
    let frame = asset_card_frame(ui);
    draw_card_grid(ui, id_salt, frame, add_cards);
}

/// The top of an asset card: its title and its Pack Asset or Stop button, its progress, then its description and
/// source. Returns whether Pack Asset and Stop were clicked.
fn draw_asset_card_header(
    ui: &mut egui::Ui,
    title: &str,
    desc: &str,
    source_note: Option<&str>,
    progress: &AssetPackProgress,
    can_start: bool,
) -> (bool, bool) {
    let mut clicked = false;
    let mut stop_clicked = false;
    let is_running = progress.state == AssetPackProgressState::Running;

    ui.horizontal(|ui| {
        // As tall as the Stop button, so the card doesn't move when its job starts.
        ui.set_min_height(STOP_BUTTON_SIZE[1]);
        ui.label(card_title(title));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if is_running {
                if ui
                    .add_sized(
                        STOP_BUTTON_SIZE,
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
    draw_asset_progress(ui, progress);
    ui.label(egui::RichText::new(desc).size(12.5).weak());
    if let Some(source_note) = source_note {
        ui.label(
            egui::RichText::new(source_note)
                .size(12.5)
                .color(SOURCE_NOTE_COLOUR),
        );
    }

    (clicked, stop_clicked)
}

/// A job's progress: a compact bar, with the percentage while the job runs, and the job's stage beside it.
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
    let is_running = progress.state == AssetPackProgressState::Running;
    ui.horizontal(|ui| {
        let bar = egui::ProgressBar::new(progress.fraction)
            .desired_width(CARD_PROGRESS_BAR_WIDTH)
            .animate(is_running);
        ui.add(if is_running { bar.show_percentage() } else { bar });
        ui.label(text);
    });
}

/// A labelled control of an option row. In a wrapping row, a cell that doesn't fit the rest of the row starts the next
/// one: `horizontal_wrapped` wraps plain widgets by their size, but not child uis like this one.
fn draw_control_cell<R>(
    ui: &mut egui::Ui,
    label: &str,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let starts_row = ui.cursor().min.x <= ui.max_rect().left();
    if ui.layout().main_wrap() && !starts_row && ui.available_size_before_wrap().x < width {
        ui.end_row();
    }
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
                .range(ZSTD_LEVEL_RANGE),
        )
            .on_hover_text("Lower levels pack faster and larger; 19-22 can pack much slower for smaller packages. Land, art and auxiliary textures default to 19, the rest to 7.");
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
                UpscaleFilter::Xbrz2x,
                UpscaleFilter::Xbrz3x,
                UpscaleFilter::Xbrz4x,
                UpscaleFilter::SuperXbr2x,
                UpscaleFilter::Cut1_2x,
                UpscaleFilter::Cut2_2x,
                UpscaleFilter::Cut3_2x,
                UpscaleFilter::ScaleFx2x,
                UpscaleFilter::ScaleFx3x,
                UpscaleFilter::ScaleFx4x,
                UpscaleFilter::OmniScale2x,
                UpscaleFilter::OmniScale3x,
                UpscaleFilter::OmniScale4x,
                UpscaleFilter::Jinc2_2x,
                UpscaleFilter::Jinc2_3x,
                UpscaleFilter::Jinc2_4x,
                UpscaleFilter::Jinc2Sharp2x,
                UpscaleFilter::Jinc2Sharp3x,
                UpscaleFilter::Jinc2Sharp4x,
                UpscaleFilter::Jinc2Sharper2x,
                UpscaleFilter::Jinc2Sharper3x,
                UpscaleFilter::Jinc2Sharper4x,
                UpscaleFilter::Jinc2Sharpest2x,
                UpscaleFilter::Jinc2Sharpest3x,
                UpscaleFilter::Jinc2Sharpest4x,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The width of each test cell.
    const TEST_CELL_WIDTH: f32 = 200.0;
    /// The cells in the test row.
    const TEST_CELLS: usize = 3;
    /// A window wide enough for two test cells and the spacing between them, but not for three.
    const TEST_WINDOW_WIDTH: f32 = 500.0;
    /// The height of the test window.
    const TEST_WINDOW_HEIGHT: f32 = 600.0;

    #[test]
    fn option_cells_that_do_not_fit_their_row_start_the_next_one() {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(TEST_WINDOW_WIDTH, TEST_WINDOW_HEIGHT),
            )),
            ..Default::default()
        };
        let mut cells = Vec::new();
        let mut row_left = f32::NAN;
        let _ = egui::Context::default().run_ui(input, |ui| {
            ui.horizontal_wrapped(|ui| {
                row_left = ui.max_rect().left();
                for cell in 0..TEST_CELLS {
                    cells.push(draw_control_cell(ui, &format!("Cell {cell}"), TEST_CELL_WIDTH, |ui| ui.min_rect()));
                }
            });
        });

        assert_eq!(cells.len(), TEST_CELLS);
        assert_eq!(cells[0].top(), cells[1].top(), "the first two cells share a row");
        assert!(cells[1].left() > cells[0].right(), "the second cell follows the first");
        assert_eq!(cells[2].left(), row_left, "the third cell starts the next row");
        assert!(cells[2].top() > cells[0].bottom(), "below the first row");
    }
}
