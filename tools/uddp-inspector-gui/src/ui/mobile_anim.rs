use std::collections::BTreeMap;

use eframe::egui;
use udd_assets::tex_art_cc::{AtlasPackingMode, PagePixelFormat};
use udd_container::{Codec, DataType};

use crate::app::{InspectorApp, MobileAnimTreeOrder};
use crate::models::{EntryInfo, ViewMode};
use crate::ui::{metadata_grid, metadata_label};
use crate::utils::format_size;

#[derive(Clone, Copy)]
struct MobileAnimPageDetails {
    atlas_width: u32,
    atlas_height: u32,
    used_width: u32,
    used_height: u32,
    pixel_format: PagePixelFormat,
}

const MOBILE_ANIM_OVERVIEW_LEFT_WIDTH: f32 = 260.0;
const MOBILE_ANIM_OVERVIEW_RIGHT_WIDTH: f32 = 500.0;
const MOBILE_ANIM_OVERVIEW_GAP: f32 = 8.0;
const MOBILE_ANIM_DETAIL_MIN_WIDTH: f32 =
    MOBILE_ANIM_OVERVIEW_LEFT_WIDTH + MOBILE_ANIM_OVERVIEW_GAP + MOBILE_ANIM_OVERVIEW_RIGHT_WIDTH;
const MOBILE_ANIM_LIST_PANEL_DEFAULT_WIDTH: f32 = 360.0;
const MOBILE_ANIM_SECTION_HEADER_SPACING: f32 = 4.0;
const MOBILE_ANIM_PLAYBACK_PANEL_MAX_WIDTH: f32 = 560.0;
const MOBILE_ANIM_PLAYBACK_PREVIEW_GAP: f32 = 12.0;

pub fn ui_mobile_anim_cc(app: &mut InspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let Some(package) = app.mobile_anim_cc_package.clone() else {
        ui.centered_and_justified(|ui| {
            ui.label("Open a mobile_anim_cc.uddp package first.");
        });
        return;
    };

    let animations = package.animations();
    if animations.is_empty() {
        ui.label("mobile_anim_cc.uddp has no animations.");
        return;
    }

    app.selected_mobile_anim_index = app.selected_mobile_anim_index.min(animations.len() - 1);

    egui::SidePanel::left("mobile_anim_cc_list")
        .resizable(true)
        .default_width(MOBILE_ANIM_LIST_PANEL_DEFAULT_WIDTH)
        .show_inside(ui, |ui| {
            ui.heading("CC Mobile Animations");
            ui.label(format!("{} animations", animations.len()));
            ui.separator();
            ui.label("Filter:");
            ui.text_edit_singleline(&mut app.filter);
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("mobile_anim_cc_tree_order")
                    .selected_text(match app.mobile_anim_tree_order {
                        MobileAnimTreeOrder::BodyType => "Body Type",
                        MobileAnimTreeOrder::BodyId => "Body ID",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut app.mobile_anim_tree_order,
                            MobileAnimTreeOrder::BodyType,
                            "Body Type",
                        );
                        ui.selectable_value(
                            &mut app.mobile_anim_tree_order,
                            MobileAnimTreeOrder::BodyId,
                            "Body ID",
                        );
                    });
                if ui.button("Collapse All").clicked() {
                    app.mobile_anim_tree_collapse_revision =
                        app.mobile_anim_tree_collapse_revision.wrapping_add(1);
                }
            });
            ui.separator();

            let query = app.filter.to_ascii_lowercase();
            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut tree = BTreeMap::<Option<u8>, BTreeMap<u16, BTreeMap<u16, Vec<usize>>>>::new();
                let mut body_tree = BTreeMap::<u16, (Option<u8>, BTreeMap<u16, Vec<usize>>)>::new();
                for (index, animation) in animations.iter().enumerate() {
                    let frames = package.animation_frames(animation);
                    let visible_count = displayable_frame_count(
                        frames,
                        udd_assets::mobile_anim_cc::MISSING_PAGE_INDEX,
                        |frame| (frame.page_index, frame.width, frame.height),
                    );
                    let body_type = package
                        .body_type_record(animation.body_id)
                        .map(|record| record.group_type);
                    let label = cc_animation_label(animation, visible_count, frames.len());
                    let search_label = format!(
                        "{} body {} action {} {}",
                        body_type_label(body_type),
                        animation.body_id,
                        animation.action_id,
                        label,
                    );
                    if !query.is_empty() && !search_label.to_ascii_lowercase().contains(&query) {
                        continue;
                    }
                    tree.entry(body_type)
                        .or_default()
                        .entry(animation.body_id)
                        .or_default()
                        .entry(animation.action_id)
                        .or_default()
                        .push(index);
                    body_tree
                        .entry(animation.body_id)
                        .or_insert_with(|| (body_type, BTreeMap::new()))
                        .1
                        .entry(animation.action_id)
                        .or_default()
                        .push(index);
                }
                let allow_default_open = app.mobile_anim_tree_collapse_revision == 0;
                let default_open = allow_default_open && !query.is_empty();
                match app.mobile_anim_tree_order {
                    MobileAnimTreeOrder::BodyType => {
                        if tree.is_empty() {
                            ui.label("No animations match the filter.");
                        }
                        for (body_type, bodies) in tree {
                            egui::CollapsingHeader::new(body_type_label(body_type))
                                .id_salt((
                                    "mobile_anim_cc_type",
                                    app.mobile_anim_tree_collapse_revision,
                                    body_type,
                                ))
                                .default_open(allow_default_open)
                                .show(ui, |ui| {
                                    for (body_id, actions) in bodies {
                                        show_cc_body_tree(
                                            app,
                                            ctx,
                                            ui,
                                            &package,
                                            animations,
                                            body_id,
                                            actions,
                                            default_open,
                                        );
                                    }
                                });
                        }
                    }
                    MobileAnimTreeOrder::BodyId => {
                        if body_tree.is_empty() {
                            ui.label("No animations match the filter.");
                        }
                        for (body_id, (body_type, actions)) in body_tree {
                            egui::CollapsingHeader::new(format!(
                                "Body {} ({})",
                                body_id,
                                body_type_label(body_type)
                            ))
                            .id_salt((
                                "mobile_anim_cc_body_order",
                                app.mobile_anim_tree_collapse_revision,
                                body_id,
                            ))
                            .default_open(default_open)
                            .show(ui, |ui| {
                                show_cc_action_tree(
                                    app, ctx, ui, &package, animations, body_id, actions,
                                    default_open,
                                );
                            });
                        }
                    }
                }
            });
        });

    show_mobile_anim_detail_area(ui, |ui| {
        let selected_animation = animations[app.selected_mobile_anim_index];
        let frames = package.animation_frames(&selected_animation);
        apply_pending_frame_reset(
            app,
            frames,
            udd_assets::mobile_anim_cc::MISSING_PAGE_INDEX,
            |frame| (frame.page_index, frame.width, frame.height),
        );

        ui.heading("mobile_anim_cc.uddp");
        show_mobile_anim_overview(
            ui,
            [
                ("Atlas", format!("{}x{}", package.atlas_width(), package.atlas_height())),
                ("Gutter", package.gutter().to_string()),
                ("Packing", packing_mode_name(package.packing_mode()).to_string()),
                ("Page formats", page_format_summary(package.pages().iter().map(|page| page.pixel_format))),
                ("Upscale", "not stored".to_string()),
            ],
            [
                ("Textures", codec_summary(&app.entries, DataType::Texture as u8)),
                ("Metadata", codec_summary(&app.entries, DataType::Metadata as u8)),
            ],
            [
                ("Pages", package.pages().len().to_string()),
                ("Animations", package.animations().len().to_string()),
                ("Frames", package.frames().len().to_string()),
                ("Body maps", package.body_resolve().len().to_string()),
                ("Body types", package.body_types().len().to_string()),
            ],
            package.pages().iter().map(|page| {
                (page.atlas_width, page.atlas_height, page.frame_count)
            }),
        );
        ui.separator();

        metadata_grid("mobile_anim_cc_selected_animation")
            .show(ui, |ui| {
                metadata_label(ui, "Body");
                ui.label(selected_animation.body_id.to_string());
                ui.end_row();
                metadata_label(ui, "Body Type");
                ui.label(body_type_label(package.body_type_record(selected_animation.body_id).map(|record| record.group_type)));
                ui.end_row();
                metadata_label(ui, "Action");
                ui.label(selected_animation.action_id.to_string());
                ui.end_row();
                metadata_label(ui, "Direction");
                ui.label(selected_animation.direction.to_string());
                ui.end_row();
                metadata_label(ui, "Source File");
                ui.label(format!("anim{}", selected_animation.file_index + 1));
                ui.end_row();
                metadata_label(ui, "Source Index");
                ui.label(selected_animation.source_index.to_string());
                ui.end_row();
                metadata_label(ui, "Flags");
                ui.label(format!("0x{:04X}", selected_animation.flags));
                ui.end_row();
            });

        ui.separator();
        if let Some(frame) = frames.get(app.selected_mobile_anim_frame_index).copied() {
            let page_size = cc_frame_page_size(&package, frame);
            show_frame_playback_preview_row(ui, app, |ui, app| {
                show_playback_controls(ctx, ui, app, frames.len());
                show_cc_frame_metadata(ui, frame, page_size);
            }, |ui, app| {
                show_cc_frame_image(ui, ctx, app, &package, frame, page_size);
            });
        } else {
            show_playback_controls(ctx, ui, app, frames.len());
        }
    });
}

fn show_cc_body_tree(
    app: &mut InspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    package: &udd_assets::MobileAnimCcPackage,
    animations: &[udd_assets::mobile_anim_cc::MobileAnimCcAnimationRecord],
    body_id: u16,
    actions: BTreeMap<u16, Vec<usize>>,
    default_open: bool,
) {
    egui::CollapsingHeader::new(format!("Body {}", body_id))
        .id_salt((
            "mobile_anim_cc_body",
            app.mobile_anim_tree_collapse_revision,
            body_id,
        ))
        .default_open(default_open)
        .show(ui, |ui| {
            show_cc_action_tree(
                app, ctx, ui, package, animations, body_id, actions, default_open,
            );
        });
}

fn show_cc_action_tree(
    app: &mut InspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    package: &udd_assets::MobileAnimCcPackage,
    animations: &[udd_assets::mobile_anim_cc::MobileAnimCcAnimationRecord],
    body_id: u16,
    actions: BTreeMap<u16, Vec<usize>>,
    default_open: bool,
) {
    for (action_id, indexes) in actions {
        egui::CollapsingHeader::new(format!("Action {}", action_id))
            .id_salt((
                "mobile_anim_cc_action",
                app.mobile_anim_tree_collapse_revision,
                body_id,
                action_id,
            ))
            .default_open(default_open)
            .show(ui, |ui| {
                for index in indexes {
                    let animation = &animations[index];
                    let frames = package.animation_frames(animation);
                    let visible_count = displayable_frame_count(
                        frames,
                        udd_assets::mobile_anim_cc::MISSING_PAGE_INDEX,
                        |frame| (frame.page_index, frame.width, frame.height),
                    );
                    let label = cc_animation_label(animation, visible_count, frames.len());
                    let is_selected = app.selected_mobile_anim_index == index;
                    let resp = ui.selectable_label(is_selected, label);
                    if is_selected && app.scroll_to_selected {
                        resp.scroll_to_me(None);
                        app.scroll_to_selected = false;
                    }
                    if resp.clicked() {
                        select_mobile_animation(app, ctx, index);
                    }
                }
            });
    }
}

pub fn ui_mobile_anim_ec(app: &mut InspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let Some(package) = app.mobile_anim_ec_package.clone() else {
        ui.centered_and_justified(|ui| {
            ui.label("Open a mobile_anim_ec.uddp package first.");
        });
        return;
    };

    let animations = package.animations();
    if animations.is_empty() {
        ui.label("mobile_anim_ec.uddp has no animations.");
        return;
    }

    app.selected_mobile_anim_index = app.selected_mobile_anim_index.min(animations.len() - 1);

    egui::SidePanel::left("mobile_anim_ec_list")
        .resizable(true)
        .default_width(MOBILE_ANIM_LIST_PANEL_DEFAULT_WIDTH)
        .show_inside(ui, |ui| {
            ui.heading("EC Mobile Animations");
            ui.label(format!("{} animations", animations.len()));
            ui.separator();
            ui.label("Filter:");
            ui.text_edit_singleline(&mut app.filter);
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("mobile_anim_ec_tree_order")
                    .selected_text(match app.mobile_anim_tree_order {
                        MobileAnimTreeOrder::BodyType => "Body Type",
                        MobileAnimTreeOrder::BodyId => "Body ID",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut app.mobile_anim_tree_order,
                            MobileAnimTreeOrder::BodyType,
                            "Body Type",
                        );
                        ui.selectable_value(
                            &mut app.mobile_anim_tree_order,
                            MobileAnimTreeOrder::BodyId,
                            "Body ID",
                        );
                    });
                if ui.button("Collapse All").clicked() {
                    app.mobile_anim_tree_collapse_revision =
                        app.mobile_anim_tree_collapse_revision.wrapping_add(1);
                }
            });
            ui.separator();

            let query = app.filter.to_ascii_lowercase();
            egui::ScrollArea::vertical().show(ui, |ui| {
                let mut tree = BTreeMap::<Option<i16>, BTreeMap<u32, BTreeMap<u16, Vec<usize>>>>::new();
                let mut body_tree = BTreeMap::<u32, (Option<i16>, BTreeMap<u16, Vec<usize>>)>::new();
                for (index, animation) in animations.iter().enumerate() {
                    let frames = package.animation_frames(animation);
                    let visible_count = displayable_frame_count(
                        frames,
                        udd_assets::mobile_anim_ec::MISSING_PAGE_INDEX,
                        |frame| (frame.page_index, frame.width, frame.height),
                    );
                    let body_type = ec_body_type_for_body(&package, animation.body_id);
                    let label = format!(
                        "{} body {} action {} {}",
                        ec_body_type_label(body_type),
                        animation.body_id,
                        animation.action_id,
                        ec_animation_label(animation, visible_count, frames.len()),
                    );
                    if !query.is_empty() && !label.to_ascii_lowercase().contains(&query) {
                        continue;
                    }
                    tree.entry(body_type)
                        .or_default()
                        .entry(animation.body_id)
                        .or_default()
                        .entry(animation.action_id)
                        .or_default()
                        .push(index);
                    body_tree
                        .entry(animation.body_id)
                        .or_insert_with(|| (body_type, BTreeMap::new()))
                        .1
                        .entry(animation.action_id)
                        .or_default()
                        .push(index);
                }
                let allow_default_open = app.mobile_anim_tree_collapse_revision == 0;
                let default_open = allow_default_open && !query.is_empty();
                match app.mobile_anim_tree_order {
                    MobileAnimTreeOrder::BodyType => {
                        if tree.is_empty() {
                            ui.label("No animations match the filter.");
                        }
                        for (body_type, bodies) in tree {
                            egui::CollapsingHeader::new(ec_body_type_label(body_type))
                                .id_salt((
                                    "mobile_anim_ec_type",
                                    app.mobile_anim_tree_collapse_revision,
                                    body_type,
                                ))
                                .default_open(allow_default_open)
                                .show(ui, |ui| {
                                    for (body_id, actions) in bodies {
                                        show_ec_body_tree(
                                            app,
                                            ctx,
                                            ui,
                                            &package,
                                            animations,
                                            body_id,
                                            actions,
                                            default_open,
                                        );
                                    }
                                });
                        }
                    }
                    MobileAnimTreeOrder::BodyId => {
                        if body_tree.is_empty() {
                            ui.label("No animations match the filter.");
                        }
                        for (body_id, (body_type, actions)) in body_tree {
                            egui::CollapsingHeader::new(format!(
                                "Body {} ({})",
                                body_id,
                                ec_body_type_label(body_type)
                            ))
                            .id_salt((
                                "mobile_anim_ec_body_order",
                                app.mobile_anim_tree_collapse_revision,
                                body_id,
                            ))
                            .default_open(default_open)
                            .show(ui, |ui| {
                                show_ec_action_tree(
                                    app, ctx, ui, &package, animations, body_id, actions,
                                    default_open,
                                );
                            });
                        }
                    }
                }
            });
        });

    show_mobile_anim_detail_area(ui, |ui| {
        let selected_animation = animations[app.selected_mobile_anim_index];
        let frames = package.animation_frames(&selected_animation);
        apply_pending_frame_reset(
            app,
            frames,
            udd_assets::mobile_anim_ec::MISSING_PAGE_INDEX,
            |frame| (frame.page_index, frame.width, frame.height),
        );

        ui.heading("mobile_anim_ec.uddp");
        show_mobile_anim_overview(
            ui,
            [
                ("Atlas", format!("{}x{}", package.atlas_width(), package.atlas_height())),
                ("Gutter", package.gutter().to_string()),
                ("Packing", packing_mode_name(package.packing_mode()).to_string()),
                ("Page formats", page_format_summary(package.pages().iter().map(|page| page.pixel_format))),
                ("Upscale", "not stored".to_string()),
            ],
            [
                ("Textures", codec_summary(&app.entries, DataType::Texture as u8)),
                ("Metadata", codec_summary(&app.entries, DataType::Metadata as u8)),
            ],
            [
                ("Pages", package.pages().len().to_string()),
                ("Animations", package.animations().len().to_string()),
                ("Frames", package.frames().len().to_string()),
                ("Items", package.items().len().to_string()),
                ("Source hints", package.source_hints().len().to_string()),
            ],
            package.pages().iter().map(|page| {
                (page.atlas_width, page.atlas_height, page.frame_count)
            }),
        );
        ui.separator();

        metadata_grid("mobile_anim_ec_selected_animation")
            .show(ui, |ui| {
                metadata_label(ui, "Body");
                ui.label(selected_animation.body_id.to_string());
                ui.end_row();
                metadata_label(ui, "Body Type");
                ui.label(ec_body_type_label(ec_body_type_for_body(&package, selected_animation.body_id)));
                ui.end_row();
                metadata_label(ui, "Action");
                ui.label(selected_animation.action_id.to_string());
                ui.end_row();
                metadata_label(ui, "Direction");
                ui.label(selected_animation.direction.to_string());
                ui.end_row();
                metadata_label(ui, "Flags");
                ui.label(format!("0x{:04X}", selected_animation.flags));
                ui.end_row();
            });

        ui.separator();
        if let Some(frame) = frames.get(app.selected_mobile_anim_frame_index).copied() {
            let page_size = ec_frame_page_size(&package, frame);
            show_frame_playback_preview_row(ui, app, |ui, app| {
                show_playback_controls(ctx, ui, app, frames.len());
                show_ec_frame_metadata(ui, frame, page_size);
            }, |ui, app| {
                show_ec_frame_image(ui, ctx, app, &package, frame, page_size);
            });
        } else {
            show_playback_controls(ctx, ui, app, frames.len());
        }
    });
}

fn show_ec_body_tree(
    app: &mut InspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    package: &udd_assets::MobileAnimEcPackage,
    animations: &[udd_assets::mobile_anim_ec::MobileAnimEcAnimationRecord],
    body_id: u32,
    actions: BTreeMap<u16, Vec<usize>>,
    default_open: bool,
) {
    egui::CollapsingHeader::new(format!("Body {}", body_id))
        .id_salt((
            "mobile_anim_ec_body",
            app.mobile_anim_tree_collapse_revision,
            body_id,
        ))
        .default_open(default_open)
        .show(ui, |ui| {
            show_ec_action_tree(
                app, ctx, ui, package, animations, body_id, actions, default_open,
            );
        });
}

fn show_ec_action_tree(
    app: &mut InspectorApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    package: &udd_assets::MobileAnimEcPackage,
    animations: &[udd_assets::mobile_anim_ec::MobileAnimEcAnimationRecord],
    body_id: u32,
    actions: BTreeMap<u16, Vec<usize>>,
    default_open: bool,
) {
    for (action_id, indexes) in actions {
        egui::CollapsingHeader::new(format!("Action {}", action_id))
            .id_salt((
                "mobile_anim_ec_action",
                app.mobile_anim_tree_collapse_revision,
                body_id,
                action_id,
            ))
            .default_open(default_open)
            .show(ui, |ui| {
                for index in indexes {
                    let animation = &animations[index];
                    let frames = package.animation_frames(animation);
                    let visible_count = displayable_frame_count(
                        frames,
                        udd_assets::mobile_anim_ec::MISSING_PAGE_INDEX,
                        |frame| (frame.page_index, frame.width, frame.height),
                    );
                    let label = ec_animation_label(animation, visible_count, frames.len());
                    let is_selected = app.selected_mobile_anim_index == index;
                    let resp = ui.selectable_label(is_selected, label);
                    if is_selected && app.scroll_to_selected {
                        resp.scroll_to_me(None);
                        app.scroll_to_selected = false;
                    }
                    if resp.clicked() {
                        select_mobile_animation(app, ctx, index);
                    }
                }
            });
    }
}

fn clamp_selected_frame(app: &mut InspectorApp, frame_count: usize) {
    if frame_count == 0 {
        app.selected_mobile_anim_frame_index = 0;
    } else {
        app.selected_mobile_anim_frame_index =
            app.selected_mobile_anim_frame_index.min(frame_count - 1);
    }
}

fn apply_pending_frame_reset<T>(
    app: &mut InspectorApp,
    frames: &[T],
    missing_page_index: u32,
    frame_parts: impl Fn(&T) -> (u32, u16, u16),
) {
    if app.mobile_anim_frame_reset_pending {
        app.selected_mobile_anim_frame_index =
            first_displayable_frame_index(frames, missing_page_index, frame_parts).unwrap_or(0);
        app.mobile_anim_frame_reset_pending = false;
    }
    clamp_selected_frame(app, frames.len());
}

fn first_displayable_frame_index<T>(
    frames: &[T],
    missing_page_index: u32,
    frame_parts: impl Fn(&T) -> (u32, u16, u16),
) -> Option<usize> {
    frames.iter().position(|frame| {
        let (page_index, width, height) = frame_parts(frame);
        is_displayable_frame(page_index, width, height, missing_page_index)
    })
}

fn displayable_frame_count<T>(
    frames: &[T],
    missing_page_index: u32,
    frame_parts: impl Fn(&T) -> (u32, u16, u16),
) -> usize {
    frames
        .iter()
        .filter(|frame| {
            let (page_index, width, height) = frame_parts(frame);
            is_displayable_frame(page_index, width, height, missing_page_index)
        })
        .count()
}

fn is_displayable_frame(
    page_index: u32,
    width: u16,
    height: u16,
    missing_page_index: u32,
) -> bool {
    page_index != missing_page_index && width != 0 && height != 0
}

fn show_playback_controls(
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    app: &mut InspectorApp,
    frame_count: usize,
) {
    if frame_count == 0 {
        ui.label("Animation has no frame records.");
        return;
    }

    if app.mobile_anim_is_playing {
        let time = ctx.input(|input| input.time);
        let frame_delay = 0.1 / app.mobile_anim_playback_speed.max(0.1) as f64;
        if time - app.mobile_anim_last_frame_time >= frame_delay {
            advance_frame(app, frame_count);
            app.mobile_anim_last_frame_time = time;
        }
        ctx.request_repaint();
    }

    app.selected_mobile_anim_frame_index =
        app.selected_mobile_anim_frame_index.min(frame_count - 1);

    ui.horizontal_wrapped(|ui| {
        if ui
            .button(if app.mobile_anim_is_playing { "Stop" } else { "Play" })
            .clicked()
        {
            app.mobile_anim_is_playing = !app.mobile_anim_is_playing;
            app.mobile_anim_last_frame_time = ctx.input(|input| input.time);
        }
        if ui.button("First").clicked() {
            app.selected_mobile_anim_frame_index = 0;
            app.mobile_anim_last_frame_time = ctx.input(|input| input.time);
        }
        if ui.button("Prev").clicked() {
            app.selected_mobile_anim_frame_index =
                app.selected_mobile_anim_frame_index.saturating_sub(1);
            app.mobile_anim_last_frame_time = ctx.input(|input| input.time);
        }
        if ui.button("Next").clicked() {
            advance_frame(app, frame_count);
            app.mobile_anim_last_frame_time = ctx.input(|input| input.time);
        }
        ui.checkbox(&mut app.mobile_anim_loop, "Loop");
        ui.label(format!(
            "Frame {} / {}",
            app.selected_mobile_anim_frame_index + 1,
            frame_count
        ));
    });

    let mut frame_index = app.selected_mobile_anim_frame_index;
    if ui
        .add(egui::Slider::new(&mut frame_index, 0..=frame_count - 1).text("Frame"))
        .changed()
    {
        app.selected_mobile_anim_frame_index = frame_index;
        app.mobile_anim_last_frame_time = ctx.input(|input| input.time);
    }
    ui.add(egui::Slider::new(&mut app.mobile_anim_playback_speed, 0.1..=5.0).text("Speed"));

    ui.horizontal_wrapped(|ui| {
        ui.label("Quick frames:");
        for index in 0..frame_count.min(32) {
            if ui
                .selectable_label(app.selected_mobile_anim_frame_index == index, index.to_string())
                .clicked()
            {
                app.selected_mobile_anim_frame_index = index;
                app.mobile_anim_last_frame_time = ctx.input(|input| input.time);
            }
        }
        if frame_count > 32 {
            ui.label(format!("... {} total", frame_count));
        }
    });
}

fn advance_frame(app: &mut InspectorApp, frame_count: usize) {
    app.selected_mobile_anim_frame_index += 1;
    if app.selected_mobile_anim_frame_index >= frame_count {
        if app.mobile_anim_loop {
            app.selected_mobile_anim_frame_index = 0;
        } else {
            app.selected_mobile_anim_frame_index = frame_count - 1;
            app.mobile_anim_is_playing = false;
        }
    }
}

fn select_mobile_animation(app: &mut InspectorApp, ctx: &egui::Context, index: usize) {
    app.select_mobile_animation_entry(ctx, index);
}

impl InspectorApp {
    pub fn select_mobile_animation_entry(&mut self, ctx: &egui::Context, index: usize) {
        if self.selected_mobile_anim_index != index {
            self.selected_mobile_anim_index = index;
            self.selected_mobile_anim_frame_index = 0;
            self.mobile_anim_frame_reset_pending = true;
            self.mobile_anim_last_frame_time = ctx.input(|input| input.time);
            self.scroll_to_selected = true;
        }
    }

    pub fn filtered_mobile_anim_indices(&self) -> Vec<usize> {
        match self.view_mode {
            ViewMode::MobileAnimCc => self
                .mobile_anim_cc_package
                .as_ref()
                .map(|package| filtered_cc_mobile_anim_indices(self, package))
                .unwrap_or_default(),
            ViewMode::MobileAnimEc => self
                .mobile_anim_ec_package
                .as_ref()
                .map(|package| filtered_ec_mobile_anim_indices(self, package))
                .unwrap_or_default(),
            ViewMode::Package | ViewMode::Virtual => Vec::new(),
        }
    }
}

fn filtered_cc_mobile_anim_indices(
    app: &InspectorApp,
    package: &udd_assets::MobileAnimCcPackage,
) -> Vec<usize> {
    let query = app.filter.to_ascii_lowercase();
    let animations = package.animations();
    let mut tree = BTreeMap::<Option<u8>, BTreeMap<u16, BTreeMap<u16, Vec<usize>>>>::new();
    let mut body_tree = BTreeMap::<u16, (Option<u8>, BTreeMap<u16, Vec<usize>>)>::new();
    for (index, animation) in animations.iter().enumerate() {
        let frames = package.animation_frames(animation);
        let visible_count = displayable_frame_count(
            frames,
            udd_assets::mobile_anim_cc::MISSING_PAGE_INDEX,
            |frame| (frame.page_index, frame.width, frame.height),
        );
        let body_type = package
            .body_type_record(animation.body_id)
            .map(|record| record.group_type);
        let label = cc_animation_label(animation, visible_count, frames.len());
        let search_label = format!(
            "{} body {} action {} {}",
            body_type_label(body_type),
            animation.body_id,
            animation.action_id,
            label,
        );
        if !query.is_empty() && !search_label.to_ascii_lowercase().contains(&query) {
            continue;
        }
        tree.entry(body_type)
            .or_default()
            .entry(animation.body_id)
            .or_default()
            .entry(animation.action_id)
            .or_default()
            .push(index);
        body_tree
            .entry(animation.body_id)
            .or_insert_with(|| (body_type, BTreeMap::new()))
            .1
            .entry(animation.action_id)
            .or_default()
            .push(index);
    }
    match app.mobile_anim_tree_order {
        MobileAnimTreeOrder::BodyType => tree
            .into_values()
            .flat_map(|bodies| bodies.into_values())
            .flat_map(|actions| actions.into_values())
            .flatten()
            .collect(),
        MobileAnimTreeOrder::BodyId => body_tree
            .into_values()
            .flat_map(|(_, actions)| actions.into_values())
            .flatten()
            .collect(),
    }
}

fn filtered_ec_mobile_anim_indices(
    app: &InspectorApp,
    package: &udd_assets::MobileAnimEcPackage,
) -> Vec<usize> {
    let query = app.filter.to_ascii_lowercase();
    let animations = package.animations();
    let mut tree = BTreeMap::<Option<i16>, BTreeMap<u32, BTreeMap<u16, Vec<usize>>>>::new();
    let mut body_tree = BTreeMap::<u32, (Option<i16>, BTreeMap<u16, Vec<usize>>)>::new();
    for (index, animation) in animations.iter().enumerate() {
        let frames = package.animation_frames(animation);
        let visible_count = displayable_frame_count(
            frames,
            udd_assets::mobile_anim_ec::MISSING_PAGE_INDEX,
            |frame| (frame.page_index, frame.width, frame.height),
        );
        let body_type = ec_body_type_for_body(package, animation.body_id);
        let label = format!(
            "{} body {} action {} {}",
            ec_body_type_label(body_type),
            animation.body_id,
            animation.action_id,
            ec_animation_label(animation, visible_count, frames.len()),
        );
        if !query.is_empty() && !label.to_ascii_lowercase().contains(&query) {
            continue;
        }
        tree.entry(body_type)
            .or_default()
            .entry(animation.body_id)
            .or_default()
            .entry(animation.action_id)
            .or_default()
            .push(index);
        body_tree
            .entry(animation.body_id)
            .or_insert_with(|| (body_type, BTreeMap::new()))
            .1
            .entry(animation.action_id)
            .or_default()
            .push(index);
    }
    match app.mobile_anim_tree_order {
        MobileAnimTreeOrder::BodyType => tree
            .into_values()
            .flat_map(|bodies| bodies.into_values())
            .flat_map(|actions| actions.into_values())
            .flatten()
            .collect(),
        MobileAnimTreeOrder::BodyId => body_tree
            .into_values()
            .flat_map(|(_, actions)| actions.into_values())
            .flatten()
            .collect(),
    }
}

fn cc_animation_label(
    animation: &udd_assets::mobile_anim_cc::MobileAnimCcAnimationRecord,
    visible_count: usize,
    frame_count: usize,
) -> String {
    format!(
        "dir {} file {} idx {} | {}/{} visible",
        animation.direction,
        animation.file_index,
        animation.source_index,
        visible_count,
        frame_count
    )
}

fn ec_animation_label(
    animation: &udd_assets::mobile_anim_ec::MobileAnimEcAnimationRecord,
    visible_count: usize,
    frame_count: usize,
) -> String {
    format!(
        "dir {} | {}/{} visible",
        animation.direction,
        visible_count,
        frame_count
    )
}

fn body_type_label(body_type: Option<u8>) -> String {
    match body_type {
        Some(body_type) => format!("Body type {}", body_type),
        None => "Body type not stored".to_string(),
    }
}

fn ec_body_type_for_body(package: &udd_assets::MobileAnimEcPackage, body_id: u32) -> Option<i16> {
    let item_id = i32::try_from(body_id).ok()?;
    package
        .items()
        .iter()
        .find(|item| item.item_id == item_id)
        .map(|item| item.item_type)
}

fn ec_body_type_label(body_type: Option<i16>) -> String {
    match body_type {
        Some(body_type) => format!("Body type {}", body_type),
        None => "Body type not stored".to_string(),
    }
}

fn show_mobile_anim_detail_area(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(MOBILE_ANIM_DETAIL_MIN_WIDTH);
            content(ui);
        });
}

fn show_mobile_anim_overview(
    ui: &mut egui::Ui,
    atlas_rows: impl IntoIterator<Item = (&'static str, String)>,
    storage_rows: impl IntoIterator<Item = (&'static str, String)>,
    content_rows: impl IntoIterator<Item = (&'static str, String)>,
    pages: impl Iterator<Item = (u32, u32, u32)>,
) {
    let page_buckets = page_bucket_rows(pages);
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            show_metadata_section(
                ui,
                "mobile_anim_overview_atlas",
                "Atlas",
                atlas_rows,
                MOBILE_ANIM_OVERVIEW_LEFT_WIDTH,
            );
            ui.add_space(MOBILE_ANIM_OVERVIEW_GAP);
            show_metadata_section(
                ui,
                "mobile_anim_overview_storage",
                "Storage",
                storage_rows,
                MOBILE_ANIM_OVERVIEW_RIGHT_WIDTH,
            );
        });
        ui.horizontal(|ui| {
            show_metadata_section(
                ui,
                "mobile_anim_overview_content",
                "Content",
                content_rows,
                MOBILE_ANIM_OVERVIEW_LEFT_WIDTH,
            );
            ui.add_space(MOBILE_ANIM_OVERVIEW_GAP);
            show_page_bucket_section(ui, &page_buckets, MOBILE_ANIM_OVERVIEW_RIGHT_WIDTH);
        });
    });
}

fn show_metadata_section(
    ui: &mut egui::Ui,
    id: &'static str,
    title: &'static str,
    rows: impl IntoIterator<Item = (&'static str, String)>,
    width: f32,
) {
    ui.group(|ui| {
        ui.set_min_width(width);
        ui.set_max_width(width);
        ui.strong(title);
        ui.add_space(MOBILE_ANIM_SECTION_HEADER_SPACING);
        metadata_grid(id)
            .show(ui, |ui| {
                for (label, value) in rows {
                    metadata_label(ui, label);
                    ui.label(value);
                    ui.end_row();
                }
            });
    });
}

fn show_page_bucket_section(ui: &mut egui::Ui, rows: &[(String, String)], width: f32) {
    ui.group(|ui| {
        ui.set_min_width(width);
        ui.set_max_width(width);
        ui.strong("Page Buckets");
        ui.add_space(MOBILE_ANIM_SECTION_HEADER_SPACING);
        if rows.is_empty() {
            ui.label("none");
            return;
        }
        metadata_grid("mobile_anim_overview_page_buckets")
            .show(ui, |ui| {
                for (size, summary) in rows {
                    metadata_label(ui, size.as_str());
                    ui.label(summary);
                    ui.end_row();
                }
            });
    });
}

fn page_bucket_rows(pages: impl Iterator<Item = (u32, u32, u32)>) -> Vec<(String, String)> {
    let mut buckets = BTreeMap::<(u32, u32), (u32, u32)>::new();
    for (width, height, frame_count) in pages {
        let entry = buckets.entry((width, height)).or_default();
        entry.0 += 1;
        entry.1 += frame_count;
    }
    buckets
        .into_iter()
        .map(|((width, height), (page_count, frame_count))| {
            (
                format!("{width}x{height}"),
                format!("{page_count} pages / {frame_count} frames"),
            )
        })
        .collect()
}

fn packing_mode_name(mode: AtlasPackingMode) -> &'static str {
    match mode {
        AtlasPackingMode::MaximumPacking => "maximum packing",
        AtlasPackingMode::Bc7Oriented => "BC7 oriented",
    }
}

fn pixel_format_name(format: PagePixelFormat) -> &'static str {
    match format {
        PagePixelFormat::Rgba8888 => "rgba8888",
        PagePixelFormat::Bc7 => "bc7",
    }
}

fn codec_name(codec: Codec) -> &'static str {
    match codec {
        Codec::None => "none",
        Codec::ZstdNoDict => "zstd",
        Codec::ZstdTypeDict => "zstd type dict",
        Codec::JpegXl => "jpegxl",
    }
}

fn page_format_summary(formats: impl Iterator<Item = PagePixelFormat>) -> String {
    let mut buckets = BTreeMap::<&'static str, usize>::new();
    for format in formats {
        *buckets.entry(pixel_format_name(format)).or_default() += 1;
    }
    if buckets.is_empty() {
        return "none".to_string();
    }
    buckets
        .into_iter()
        .map(|(format, count)| format!("{format}: {count} pages"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn codec_summary(entries: &[EntryInfo], data_type: u8) -> String {
    let mut buckets = BTreeMap::<&'static str, (usize, u64, u64)>::new();
    for entry in entries.iter().filter(|entry| entry.data_type == data_type) {
        let bucket = buckets.entry(codec_name(entry.codec)).or_default();
        bucket.0 += 1;
        bucket.1 += entry.raw_size as u64;
        bucket.2 += entry.stored_size as u64;
    }
    if buckets.is_empty() {
        return "none".to_string();
    }
    buckets
        .into_iter()
        .map(|(codec, (count, raw_size, stored_size))| {
            format!(
                "{codec}: {count} entries, {} raw / {} stored",
                format_size(raw_size),
                format_size(stored_size)
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn show_frame_playback_preview_row(
    ui: &mut egui::Ui,
    app: &mut InspectorApp,
    controls_and_metadata: impl FnOnce(&mut egui::Ui, &mut InspectorApp),
    preview: impl FnOnce(&mut egui::Ui, &mut InspectorApp),
) {
    ui.horizontal_wrapped(|ui| {
        ui.vertical(|ui| {
            ui.set_width(ui.available_width().min(MOBILE_ANIM_PLAYBACK_PANEL_MAX_WIDTH));
            controls_and_metadata(ui, app);
        });
        ui.add_space(MOBILE_ANIM_PLAYBACK_PREVIEW_GAP);
        ui.vertical(|ui| {
            preview(ui, app);
        });
    });
}

fn show_cc_frame_metadata(
    ui: &mut egui::Ui,
    frame: udd_assets::mobile_anim_cc::MobileAnimCcFrameRecord,
    page_size: Option<MobileAnimPageDetails>,
) {
    show_frame_metadata(
        ui,
        frame.frame_index,
        frame.frame_index,
        frame.page_index,
        frame.x,
        frame.y,
        frame.width,
        frame.height,
        frame.center_x,
        frame.center_y,
        udd_assets::mobile_anim_cc::MISSING_PAGE_INDEX,
    );
    show_page_size_metadata(ui, page_size);
}

fn show_cc_frame_image(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    app: &mut InspectorApp,
    package: &udd_assets::MobileAnimCcPackage,
    frame: udd_assets::mobile_anim_cc::MobileAnimCcFrameRecord,
    page_size: Option<MobileAnimPageDetails>,
) {
    show_frame_image(
        ui,
        ctx,
        app,
        "mobile_anim_cc",
        page_size,
        frame.page_index,
        frame.x,
        frame.y,
        frame.width,
        frame.height,
        || package.read_frame_rgba(&frame).ok(),
        || package.read_page_rgba(frame.page_index).ok(),
    );
}

fn cc_frame_page_size(
    package: &udd_assets::MobileAnimCcPackage,
    frame: udd_assets::mobile_anim_cc::MobileAnimCcFrameRecord,
) -> Option<MobileAnimPageDetails> {
    package
        .pages()
        .iter()
        .find(|page| page.page_index == frame.page_index)
        .map(|page| MobileAnimPageDetails {
            atlas_width: page.atlas_width,
            atlas_height: page.atlas_height,
            used_width: page.used_width,
            used_height: page.used_height,
            pixel_format: page.pixel_format,
        })
}

fn show_ec_frame_metadata(
    ui: &mut egui::Ui,
    frame: udd_assets::mobile_anim_ec::MobileAnimEcFrameRecord,
    page_size: Option<MobileAnimPageDetails>,
) {
    show_frame_metadata(
        ui,
        frame.frame_index,
        frame.source_frame_index,
        frame.page_index,
        frame.x,
        frame.y,
        frame.width,
        frame.height,
        frame.center_x,
        frame.center_y,
        udd_assets::mobile_anim_ec::MISSING_PAGE_INDEX,
    );
    show_page_size_metadata(ui, page_size);
}

fn show_ec_frame_image(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    app: &mut InspectorApp,
    package: &udd_assets::MobileAnimEcPackage,
    frame: udd_assets::mobile_anim_ec::MobileAnimEcFrameRecord,
    page_size: Option<MobileAnimPageDetails>,
) {
    show_frame_image(
        ui,
        ctx,
        app,
        "mobile_anim_ec",
        page_size,
        frame.page_index,
        frame.x,
        frame.y,
        frame.width,
        frame.height,
        || package.read_frame_rgba(&frame).ok(),
        || package.read_page_rgba(frame.page_index).ok(),
    );
}

fn ec_frame_page_size(
    package: &udd_assets::MobileAnimEcPackage,
    frame: udd_assets::mobile_anim_ec::MobileAnimEcFrameRecord,
) -> Option<MobileAnimPageDetails> {
    package
        .pages()
        .iter()
        .find(|page| page.page_index == frame.page_index)
        .map(|page| MobileAnimPageDetails {
            atlas_width: page.atlas_width,
            atlas_height: page.atlas_height,
            used_width: page.used_width,
            used_height: page.used_height,
            pixel_format: page.pixel_format,
        })
}

#[cfg(test)]
fn page_bucket_summary(pages: impl Iterator<Item = (u32, u32, u32)>) -> String {
    let mut buckets = BTreeMap::<(u32, u32), (u32, u32)>::new();
    for (width, height, frame_count) in pages {
        let entry = buckets.entry((width, height)).or_default();
        entry.0 += 1;
        entry.1 += frame_count;
    }
    if buckets.is_empty() {
        return "none".to_string();
    }
    buckets
        .into_iter()
        .map(|((width, height), (page_count, frame_count))| {
            format!("{width}x{height}: {page_count} pages / {frame_count} frames")
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn show_page_size_metadata(
    ui: &mut egui::Ui,
    page_size: Option<MobileAnimPageDetails>,
) {
    if let Some(page_size) = page_size {
        ui.label(format!(
            "Page size: {}x{} atlas, {}x{} used, {}",
            page_size.atlas_width,
            page_size.atlas_height,
            page_size.used_width,
            page_size.used_height,
            pixel_format_name(page_size.pixel_format)
        ));
    }
}

fn show_frame_metadata(
    ui: &mut egui::Ui,
    frame_index: u16,
    source_frame_index: u16,
    page_index: u32,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    center_x: i16,
    center_y: i16,
    missing_page_index: u32,
) {
    metadata_grid("mobile_anim_selected_frame")
        .show(ui, |ui| {
            metadata_label(ui, "Frame");
            ui.label(frame_index.to_string());
            ui.end_row();
            metadata_label(ui, "Source Frame");
            ui.label(source_frame_index.to_string());
            ui.end_row();
            metadata_label(ui, "Size");
            ui.label(format!("{}x{}", width, height));
            ui.end_row();
            metadata_label(ui, "Center");
            ui.label(format!("{},{}", center_x, center_y));
            ui.end_row();
            metadata_label(ui, "Atlas");
            if page_index == missing_page_index {
                ui.label("empty");
            } else {
                ui.label(format!("page {} rect {},{} {}x{}", page_index, x, y, width, height));
            }
            ui.end_row();
        });
}

fn show_frame_image(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    app: &mut InspectorApp,
    texture_prefix: &str,
    page_size: Option<MobileAnimPageDetails>,
    page_index: u32,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    read_frame_rgba: impl Fn() -> Option<Vec<u8>>,
    read_page_rgba: impl Fn() -> Option<Vec<u8>>,
) {
    if width == 0 || height == 0 {
        ui.label("(Empty frame)");
        return;
    }

    let size = [width as usize, height as usize];
    let label = format!("{texture_prefix} page {page_index} rect {x},{y} {width}x{height}");

    if app.preview_texture.is_none()
        || app.preview_texture_size != Some(size)
        || app.preview_text.as_deref() != Some(label.as_str())
    {
        let Some(cropped) = read_frame_rgba() else {
            ui.label("Unable to read frame image.");
            return;
        };
        if cropped.len() != size[0] * size[1] * 4 {
            ui.label("Frame image has an unexpected size.");
            return;
        }

        app.set_preview_image(
            ctx,
            &format!("{texture_prefix}_page{page_index}_{x}_{y}"),
            size,
            &cropped,
            label,
        );
    }

    if let Some((texture, size, _)) = app.preview_texture
        .as_ref()
        .zip(app.preview_texture_size)
        .map(|(texture, size)| {
            (
                texture,
                size,
                app.preview_text.as_deref().unwrap_or("Image Preview"),
            )
        })
    {
        let texture = texture.clone();
        let size = size;
        ui.horizontal_wrapped(|ui| {
            if ui.button("Open Frame Window").clicked() {
                app.image_window_mode = crate::models::PreviewModeKind::Entry;
                app.image_window_open = true;
            }
            if ui.button("Open Atlas Window").clicked() {
                if let Some(page_rgba) = read_page_rgba() {
                    set_mobile_anim_atlas_image(
                        ctx,
                        app,
                        texture_prefix,
                        page_index,
                        page_size,
                        &page_rgba,
                    );
                    app.image_window_mode = crate::models::PreviewModeKind::Atlas;
                    app.image_window_open = true;
                }
            }
        });
        let max = ui.available_size();
        let scale = (max.x / size[0] as f32)
            .min((max.y.max(1.0)) / size[1] as f32)
            .min(1.0)
            .max(0.1);
        egui::ScrollArea::both().show(ui, |ui| {
            ui.add(
                egui::Image::new(&texture)
                    .maintain_aspect_ratio(true)
                    .texture_options(egui::TextureOptions::NEAREST)
                    .fit_to_exact_size(egui::vec2(size[0] as f32 * scale, size[1] as f32 * scale)),
            );
        });
    }
}

fn set_mobile_anim_atlas_image(
    ctx: &egui::Context,
    app: &mut InspectorApp,
    texture_prefix: &str,
    page_index: u32,
    page_size: Option<MobileAnimPageDetails>,
    page_rgba: &[u8],
) {
    let Some(size) = decoded_page_size(page_rgba, page_size) else {
        return;
    };
    let label = page_size
        .map(|page_size| {
            format!(
                "{} page {} atlas {}x{}, used {}x{}, {}",
                texture_prefix,
                page_index,
                page_size.atlas_width,
                page_size.atlas_height,
                page_size.used_width,
                page_size.used_height,
                pixel_format_name(page_size.pixel_format)
            )
        })
        .unwrap_or_else(|| format!("{texture_prefix} page {page_index} atlas"));

    if app.atlas_texture_size == Some(size) && app.atlas_text.as_deref() == Some(label.as_str()) {
        return;
    }

    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, page_rgba);
    app.atlas_texture = Some(ctx.load_texture(
        format!("{texture_prefix}_atlas_page{page_index}"),
        color_image,
        egui::TextureOptions::NEAREST,
    ));
    app.atlas_texture_size = Some(size);
    app.atlas_text = Some(label);
}

fn decoded_page_size(
    page_rgba: &[u8],
    page_size: Option<MobileAnimPageDetails>,
) -> Option<[usize; 2]> {
    if page_rgba.len() % 4 != 0 {
        return None;
    }
    let pixels = page_rgba.len() / 4;
    if let Some(page_size) = page_size {
        let used_pixels = page_size.used_width as usize * page_size.used_height as usize;
        if pixels == used_pixels && page_size.used_width != 0 && page_size.used_height != 0 {
            return Some([page_size.used_width as usize, page_size.used_height as usize]);
        }
        let atlas_pixels = page_size.atlas_width as usize * page_size.atlas_height as usize;
        if pixels == atlas_pixels && page_size.atlas_width != 0 && page_size.atlas_height != 0 {
            return Some([page_size.atlas_width as usize, page_size.atlas_height as usize]);
        }
    }
    page_width_from_rgba_len(page_rgba).map(|side| [side as usize, side as usize])
}

fn page_width_from_rgba_len(page_rgba: &[u8]) -> Option<u32> {
    if page_rgba.len() % 4 != 0 {
        return None;
    }
    let pixels = page_rgba.len() / 4;
    let side = (pixels as f64).sqrt() as usize;
    if side != 0 && side * side == pixels {
        Some(side as u32)
    } else {
        None
    }
}

fn crop_frame_rgba(
    page_rgba: &[u8],
    page_width: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let mut cropped = vec![0u8; width as usize * height as usize * 4];
    let src_stride = page_width as usize * 4;
    let dst_stride = width as usize * 4;
    for row in 0..height as usize {
        let src_start = ((y as usize + row) * src_stride) + x as usize * 4;
        let src_end = src_start + dst_stride;
        let dst_start = row * dst_stride;
        let dst_end = dst_start + dst_stride;
        let src = page_rgba.get(src_start..src_end)?;
        cropped[dst_start..dst_end].copy_from_slice(src);
    }
    Some(cropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_frame_rgba_extracts_selected_rect() {
        let page = vec![
            1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255,
            4, 0, 0, 255, 5, 0, 0, 255, 6, 0, 0, 255,
        ];

        let cropped = crop_frame_rgba(&page, 3, 1, 0, 2, 2).unwrap();

        assert_eq!(
            cropped,
            vec![
                2, 0, 0, 255, 3, 0, 0, 255,
                5, 0, 0, 255, 6, 0, 0, 255,
            ]
        );
    }

    #[test]
    fn page_bucket_summary_groups_pages_and_frames_by_atlas_size() {
        let summary = page_bucket_summary([
            (512, 512, 10),
            (2048, 1024, 5),
            (512, 512, 3),
        ].into_iter());

        assert_eq!(
            summary,
            "512x512: 2 pages / 13 frames; 2048x1024: 1 pages / 5 frames"
        );
    }

    #[test]
    fn decoded_page_size_prefers_manifest_used_or_atlas_size() {
        let used = vec![0u8; 8 * 4 * 4];
        let atlas = vec![0u8; 16 * 16 * 4];
        let page_size = MobileAnimPageDetails {
            atlas_width: 16,
            atlas_height: 16,
            used_width: 8,
            used_height: 4,
            pixel_format: PagePixelFormat::Rgba8888,
        };

        assert_eq!(
            decoded_page_size(&used, Some(page_size)),
            Some([8, 4])
        );
        assert_eq!(
            decoded_page_size(&atlas, Some(page_size)),
            Some([16, 16])
        );
    }

    #[test]
    fn first_displayable_frame_index_skips_missing_or_empty_frames() {
        let frames = [
            (u32::MAX, 0, 0),
            (0, 0, 12),
            (0, 8, 12),
        ];

        assert_eq!(
            first_displayable_frame_index(&frames, u32::MAX, |frame| {
                (frame.0, frame.1, frame.2)
            }),
            Some(2)
        );
        assert_eq!(
            displayable_frame_count(&frames, u32::MAX, |frame| {
                (frame.0, frame.1, frame.2)
            }),
            1
        );
    }
}
