use std::collections::BTreeMap;

use eframe::egui;

use crate::app::InspectorApp;

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
    let selected_animation = animations[app.selected_mobile_anim_index];
    let frames = package.animation_frames(&selected_animation);
    clamp_selected_frame(app, frames.len());

    egui::SidePanel::left("mobile_anim_cc_list")
        .resizable(true)
        .default_width(360.0)
        .show_inside(ui, |ui| {
            ui.heading("CC Mobile Animations");
            ui.label(format!("{} animations", animations.len()));
            ui.separator();
            ui.label("Filter:");
            ui.text_edit_singleline(&mut app.filter);
            ui.separator();

            let query = app.filter.to_ascii_lowercase();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (index, animation) in animations.iter().enumerate() {
                    let label = format!(
                        "body {} action {} dir {} file {} idx {}",
                        animation.body_id,
                        animation.action_id,
                        animation.direction,
                        animation.file_index,
                        animation.source_index
                    );
                    if !query.is_empty() && !label.to_ascii_lowercase().contains(&query) {
                        continue;
                    }
                    if ui
                        .selectable_label(app.selected_mobile_anim_index == index, label)
                        .clicked()
                    {
                        app.selected_mobile_anim_index = index;
                        app.selected_mobile_anim_frame_index = 0;
                    }
                }
            });
        });

    ui.heading("mobile_anim_cc.uddp");
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("Atlas: {}x{} gutter {}", package.atlas_width(), package.atlas_height(), package.gutter()));
        ui.separator();
        ui.label(format!("Pages: {}", package.pages().len()));
        ui.separator();
        ui.label(format!("Page buckets: {}", page_bucket_summary(package.pages().iter().map(|page| {
            (page.atlas_width, page.atlas_height, page.frame_count)
        }))));
        ui.separator();
        ui.label(format!("Animations: {}", package.animations().len()));
        ui.separator();
        ui.label(format!("Frames: {}", package.frames().len()));
        ui.separator();
        ui.label(format!("Body maps: {}", package.body_resolve().len()));
        ui.separator();
        ui.label(format!("Body types: {}", package.body_types().len()));
    });
    ui.separator();

    egui::Grid::new("mobile_anim_cc_selected_animation")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Body");
            ui.label(selected_animation.body_id.to_string());
            ui.end_row();
            ui.label("Action");
            ui.label(selected_animation.action_id.to_string());
            ui.end_row();
            ui.label("Direction");
            ui.label(selected_animation.direction.to_string());
            ui.end_row();
            ui.label("Source File");
            ui.label(format!("anim{}", selected_animation.file_index + 1));
            ui.end_row();
            ui.label("Source Index");
            ui.label(selected_animation.source_index.to_string());
            ui.end_row();
            ui.label("Flags");
            ui.label(format!("0x{:04X}", selected_animation.flags));
            ui.end_row();
        });

    ui.separator();
    show_frame_selector(ui, app, frames.len());
    if let Some(frame) = frames.get(app.selected_mobile_anim_frame_index).copied() {
        show_cc_frame(ui, ctx, app, &package, frame);
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
    let selected_animation = animations[app.selected_mobile_anim_index];
    let frames = package.animation_frames(&selected_animation);
    clamp_selected_frame(app, frames.len());

    egui::SidePanel::left("mobile_anim_ec_list")
        .resizable(true)
        .default_width(360.0)
        .show_inside(ui, |ui| {
            ui.heading("EC Mobile Animations");
            ui.label(format!("{} animations", animations.len()));
            ui.separator();
            ui.label("Filter:");
            ui.text_edit_singleline(&mut app.filter);
            ui.separator();

            let query = app.filter.to_ascii_lowercase();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (index, animation) in animations.iter().enumerate() {
                    let label = format!(
                        "body {} action {} dir {}",
                        animation.body_id,
                        animation.action_id,
                        animation.direction
                    );
                    if !query.is_empty() && !label.to_ascii_lowercase().contains(&query) {
                        continue;
                    }
                    if ui
                        .selectable_label(app.selected_mobile_anim_index == index, label)
                        .clicked()
                    {
                        app.selected_mobile_anim_index = index;
                        app.selected_mobile_anim_frame_index = 0;
                    }
                }
            });
        });

    ui.heading("mobile_anim_ec.uddp");
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("Atlas: {}x{} gutter {}", package.atlas_width(), package.atlas_height(), package.gutter()));
        ui.separator();
        ui.label(format!("Pages: {}", package.pages().len()));
        ui.separator();
        ui.label(format!("Page buckets: {}", page_bucket_summary(package.pages().iter().map(|page| {
            (page.atlas_width, page.atlas_height, page.frame_count)
        }))));
        ui.separator();
        ui.label(format!("Animations: {}", package.animations().len()));
        ui.separator();
        ui.label(format!("Frames: {}", package.frames().len()));
        ui.separator();
        ui.label(format!("Items: {}", package.items().len()));
        ui.separator();
        ui.label(format!("Source hints: {}", package.source_hints().len()));
    });
    ui.separator();

    egui::Grid::new("mobile_anim_ec_selected_animation")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Body");
            ui.label(selected_animation.body_id.to_string());
            ui.end_row();
            ui.label("Action");
            ui.label(selected_animation.action_id.to_string());
            ui.end_row();
            ui.label("Direction");
            ui.label(selected_animation.direction.to_string());
            ui.end_row();
            ui.label("Flags");
            ui.label(format!("0x{:04X}", selected_animation.flags));
            ui.end_row();
        });

    ui.separator();
    show_frame_selector(ui, app, frames.len());
    if let Some(frame) = frames.get(app.selected_mobile_anim_frame_index).copied() {
        show_ec_frame(ui, ctx, app, &package, frame);
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

fn show_frame_selector(ui: &mut egui::Ui, app: &mut InspectorApp, frame_count: usize) {
    if frame_count == 0 {
        ui.label("Animation has no frame records.");
        return;
    }

    ui.horizontal_wrapped(|ui| {
        ui.label("Frame:");
        for index in 0..frame_count.min(32) {
            if ui
                .selectable_label(app.selected_mobile_anim_frame_index == index, index.to_string())
                .clicked()
            {
                app.selected_mobile_anim_frame_index = index;
            }
        }
        if frame_count > 32 {
            ui.label(format!("... {} total", frame_count));
        }
    });
}

fn show_cc_frame(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    app: &mut InspectorApp,
    package: &udd_assets::MobileAnimCcPackage,
    frame: udd_assets::mobile_anim_cc::MobileAnimCcFrameRecord,
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
    let page_width = package
        .pages()
        .iter()
        .find(|page| page.page_index == frame.page_index)
        .map(|page| page.used_width);
    let page_size = package
        .pages()
        .iter()
        .find(|page| page.page_index == frame.page_index)
        .map(|page| (page.atlas_width, page.atlas_height, page.used_width, page.used_height));
    show_page_size_metadata(ui, page_size);
    show_frame_image(ui, ctx, app, "mobile_anim_cc", page_width, frame.page_index, frame.x, frame.y, frame.width, frame.height, || {
        package.read_page_rgba(frame.page_index).ok()
    });
}

fn show_ec_frame(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    app: &mut InspectorApp,
    package: &udd_assets::MobileAnimEcPackage,
    frame: udd_assets::mobile_anim_ec::MobileAnimEcFrameRecord,
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
    let page_width = package
        .pages()
        .iter()
        .find(|page| page.page_index == frame.page_index)
        .map(|page| page.used_width);
    let page_size = package
        .pages()
        .iter()
        .find(|page| page.page_index == frame.page_index)
        .map(|page| (page.atlas_width, page.atlas_height, page.used_width, page.used_height));
    show_page_size_metadata(ui, page_size);
    show_frame_image(ui, ctx, app, "mobile_anim_ec", page_width, frame.page_index, frame.x, frame.y, frame.width, frame.height, || {
        package.read_page_rgba(frame.page_index).ok()
    });
}

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
    page_size: Option<(u32, u32, u32, u32)>,
) {
    if let Some((atlas_width, atlas_height, used_width, used_height)) = page_size {
        ui.label(format!(
            "Page size: {}x{} atlas, {}x{} used",
            atlas_width, atlas_height, used_width, used_height
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
    egui::Grid::new("mobile_anim_selected_frame")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            ui.label("Frame");
            ui.label(frame_index.to_string());
            ui.end_row();
            ui.label("Source Frame");
            ui.label(source_frame_index.to_string());
            ui.end_row();
            ui.label("Size");
            ui.label(format!("{}x{}", width, height));
            ui.end_row();
            ui.label("Center");
            ui.label(format!("{},{}", center_x, center_y));
            ui.end_row();
            ui.label("Atlas");
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
    page_width: Option<u32>,
    page_index: u32,
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    read_page_rgba: impl FnOnce() -> Option<Vec<u8>>,
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
        let Some(page_rgba) = read_page_rgba() else {
            ui.label("Unable to read frame atlas page.");
            return;
        };
        let Some(cropped) = crop_frame_rgba(
            &page_rgba,
            page_width.unwrap_or_else(|| page_width_from_rgba_len(&page_rgba).unwrap_or(2048)),
            x as u32,
            y as u32,
            width as u32,
            height as u32,
        ) else {
            ui.label("Frame rectangle is outside the atlas page.");
            return;
        };

        app.set_preview_image(
            ctx,
            &format!("{texture_prefix}_page{page_index}_{x}_{y}"),
            size,
            &cropped,
            label,
        );
    }

    if let Some((texture, size, _)) = app.active_preview_image() {
        let texture = texture.clone();
        let size = size;
        if ui.button("Open Image Window").clicked() {
            app.image_window_mode = crate::models::PreviewModeKind::Entry;
            app.image_window_open = true;
        }
        let max = ui.available_size();
        let scale = (max.x / size[0] as f32)
            .min((max.y.max(1.0)) / size[1] as f32)
            .min(1.0)
            .max(0.1);
        egui::ScrollArea::both().show(ui, |ui| {
            ui.add(
                egui::Image::new(&texture)
                    .maintain_aspect_ratio(true)
                    .fit_to_exact_size(egui::vec2(size[0] as f32 * scale, size[1] as f32 * scale)),
            );
        });
    }
}

fn page_width_from_rgba_len(page_rgba: &[u8]) -> Option<u32> {
    if page_rgba.len() % 4 != 0 {
        return None;
    }
    let pixels = page_rgba.len() / 4;
    let side = (pixels as f64).sqrt() as usize;
    if side * side == pixels {
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
}
