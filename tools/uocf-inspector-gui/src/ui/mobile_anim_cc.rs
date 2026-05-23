use eframe::egui;

use crate::app::UopInspectorApp;

pub fn ui_mobile_anim_cc(app: &mut UopInspectorApp, ctx: &egui::Context) {
    let Some(package) = app.mobile_anim_cc_package.clone() else {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label("Open a mobile_anim_cc.uddp package first");
            });
        });
        return;
    };

    let animations = package.animations();
    if animations.is_empty() {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("mobile_anim_cc package has no animations");
        });
        return;
    }

    app.selected_mobile_anim_index = app.selected_mobile_anim_index.min(animations.len() - 1);
    let selected_animation = animations[app.selected_mobile_anim_index];
    let frames = package.animation_frames(&selected_animation);
    if frames.is_empty() {
        app.selected_mobile_anim_frame_index = 0;
    } else {
        app.selected_mobile_anim_frame_index =
            app.selected_mobile_anim_frame_index.min(frames.len() - 1);
    }

    egui::SidePanel::left("mobile_anim_cc_animations")
        .resizable(true)
        .default_width(300.0)
        .show(ctx, |ui| {
            ui.heading("CC Mobile Animations");
            if let Some(path) = &app.mobile_anim_cc_path {
                ui.label(path.display().to_string());
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Filter:");
                ui.text_edit_singleline(&mut app.search_query);
            });
            ui.separator();

            let query = app.search_query.to_ascii_lowercase();
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

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("mobile_anim_cc.uddp");
        ui.horizontal(|ui| {
            ui.label(format!(
                "Atlas: {}x{} gutter {}",
                package.atlas_width(),
                package.atlas_height(),
                package.gutter()
            ));
            ui.separator();
            ui.label(format!("Pages: {}", package.pages().len()));
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
        ui.heading(format!(
            "Body {} Action {} Direction {}",
            selected_animation.body_id,
            selected_animation.action_id,
            selected_animation.direction
        ));
        egui::Grid::new("mobile_anim_cc_selected_animation")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Source File");
                ui.label(format!("anim{}", selected_animation.file_index + 1));
                ui.end_row();
                ui.label("Source Index");
                ui.label(selected_animation.source_index.to_string());
                ui.end_row();
                ui.label("Frame Range");
                ui.label(format!(
                    "{}..{}",
                    selected_animation.frame_start,
                    selected_animation.frame_start + selected_animation.frame_count as u32
                ));
                ui.end_row();
            });

        ui.separator();
        if frames.is_empty() {
            ui.label("Animation has no frame records");
            return;
        }

        ui.horizontal(|ui| {
            ui.label("Frame:");
            for index in 0..frames.len().min(16) {
                if ui
                    .selectable_label(app.selected_mobile_anim_frame_index == index, index.to_string())
                    .clicked()
                {
                    app.selected_mobile_anim_frame_index = index;
                }
            }
            if frames.len() > 16 {
                ui.label(format!("... {} total", frames.len()));
            }
        });

        let frame = frames[app.selected_mobile_anim_frame_index];
        egui::Grid::new("mobile_anim_cc_selected_frame")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Size");
                ui.label(format!("{}x{}", frame.width, frame.height));
                ui.end_row();
                ui.label("Center");
                ui.label(format!("{},{}", frame.center_x, frame.center_y));
                ui.end_row();
                ui.label("Atlas");
                if frame.page_index == udd_assets::mobile_anim_cc::MISSING_PAGE_INDEX {
                    ui.label("empty");
                } else {
                    ui.label(format!(
                        "page {} rect {},{} {}x{}",
                        frame.page_index, frame.x, frame.y, frame.width, frame.height
                    ));
                }
                ui.end_row();
            });

        if frame.width == 0
            || frame.height == 0
            || frame.page_index == udd_assets::mobile_anim_cc::MISSING_PAGE_INDEX
        {
            ui.label("(Empty frame)");
            return;
        }

        match frame_texture(app, ctx, &package, frame) {
            Some(handle) => {
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.image(&handle);
                });
            }
            None => {
                ui.label("Unable to read frame texture");
            }
        }
    });
}

fn frame_texture(
    app: &mut UopInspectorApp,
    ctx: &egui::Context,
    package: &udd_assets::MobileAnimCcPackage,
    frame: udd_assets::mobile_anim_cc::MobileAnimCcFrameRecord,
) -> Option<egui::TextureHandle> {
    let key = 0xAC00000000000000u64
        | ((frame.page_index as u64) << 32)
        | ((frame.x as u64) << 16)
        | frame.y as u64;
    if let Some(handle) = app.texture_previews.get(&key) {
        return Some(handle.clone());
    }

    let page_rgba = package.read_page_rgba(frame.page_index).ok()?;
    let cropped = crop_frame_rgba(
        &page_rgba,
        package.atlas_width(),
        frame.x as u32,
        frame.y as u32,
        frame.width as u32,
        frame.height as u32,
    )?;
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [frame.width as usize, frame.height as usize],
        &cropped,
    );
    let handle = ctx.load_texture(
        format!(
            "mobile_anim_cc_page{}_{}_{}",
            frame.page_index, frame.x, frame.y
        ),
        image,
        Default::default(),
    );
    app.texture_previews.insert(key, handle.clone());
    Some(handle)
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
    fn crop_frame_rgba_rejects_out_of_bounds_rect() {
        let page = vec![0u8; 2 * 2 * 4];

        assert!(crop_frame_rgba(&page, 2, 1, 1, 2, 1).is_none());
    }
}
