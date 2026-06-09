use eframe::egui;
use crate::app::UddConvApp;

const PREVIEW_WINDOW_WIDTH: f32 = 800.0;
const PREVIEW_WINDOW_HEIGHT: f32 = 800.0;
const UPSCALE_PREVIEW_WINDOW_WIDTH: f32 = 1000.0;
const UPSCALE_PREVIEW_WINDOW_HEIGHT: f32 = 800.0;

impl UddConvApp {
    pub fn ui_preview_window(&mut self, ctx: &egui::Context) {
        let Some(path) = self.preview_path.clone() else {
            return;
        };

        let mut open = true;
        let preview_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        egui::Window::new(format!(
            "Preview: {}",
            preview_name
        ))
        .open(&mut open)
        .resizable(true)
        .default_size([PREVIEW_WINDOW_WIDTH, PREVIEW_WINDOW_HEIGHT])
        .show(ctx, |ui| {
            if self.preview_texture.is_none() {
                self.load_preview_texture(ctx, &path);
            }

            if let Some(texture) = &self.preview_texture {
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.image(texture);
                });
            }
        });

        if !open {
            self.preview_path = None;
            self.preview_texture = None;
        }
    }

    fn load_preview_texture(&mut self, ctx: &egui::Context, path: &std::path::Path) {
        let Ok(data) = std::fs::read(path) else {
            return;
        };

        if path.extension().is_some_and(|ext| ext == "ktx2") {
            if let Some(color_image) = self.load_ktx2_preview(&data) {
                self.preview_texture = Some(ctx.load_texture("preview", color_image, Default::default()));
            }
            return;
        }

        if let Ok(image) = image::load_from_memory(&data) {
            let size = [image.width() as usize, image.height() as usize];
            let color_image = egui::ColorImage::from_rgba_unmultiplied(
                size,
                image.to_rgba8().as_flat_samples().as_slice(),
            );
            self.preview_texture = Some(ctx.load_texture("preview", color_image, Default::default()));
        }
    }

    fn load_ktx2_preview(&self, data: &[u8]) -> Option<egui::ColorImage> {
        let reader = ktx2::Reader::new(data).ok()?;
        let header = reader.header();

        if header.format != Some(ktx2::Format::BC7_UNORM_BLOCK) {
            return None;
        }

        let level0 = reader.levels().next()?;
        let mut blocks = level0.data.to_vec();

        if header.supercompression_scheme == Some(ktx2::SupercompressionScheme::Zstandard) {
            blocks = zstd::decode_all(std::io::Cursor::new(&blocks)).ok()?;
        }

        let width = header.pixel_width;
        let height = header.pixel_height;
        let extent = udd_conv::bc7::ImageExtent::new(width, height).ok()?;
        let rgba = udd_conv::bc7::decode_bc7_to_rgba8888(&blocks, extent).ok()?;

        Some(egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            &rgba,
        ))
    }

    pub fn ui_upscale_preview_window(&mut self, ctx: &egui::Context) {
        if self.upscale_preview.is_none() {
            return;
        }

        self.update_upscale_preview(ctx);

        let Some(preview) = self.upscale_preview.as_mut() else {
            return;
        };
        let mut open = true;
        let mut is_dirty = false;
        let mut id = preview.id;
        let mut id_buffer = preview.id_buffer.clone();
        let target = preview.target;
        let filter = preview.filter;
        let original_size = preview.original_size;
        let upscaled_size = preview.upscaled_size;
        let texture = preview.texture.clone();
        let upscaled_texture = preview.upscaled_texture.clone();
        let mut zoom = preview.zoom;

        egui::Window::new(format!("Upscale Preview: {:?}", target))
            .open(&mut open)
            .resizable(true)
            .default_size([UPSCALE_PREVIEW_WINDOW_WIDTH, UPSCALE_PREVIEW_WINDOW_HEIGHT])
            .show(ctx, |ui| {
                // Toolbar
                ui.horizontal(|ui| {
                    ui.label("Item ID:");
                    if ui.button("◀").clicked() {
                        id = id.saturating_sub(1);
                        id_buffer = id.to_string();
                        is_dirty = true;
                    }

                    let resp =
                        ui.add(egui::TextEdit::singleline(&mut id_buffer).desired_width(60.0));
                    if resp.changed() {
                        if let Ok(new_id) = id_buffer.parse::<u32>() {
                            id = new_id;
                            is_dirty = true;
                        }
                    }

                    if ui.button("▶").clicked() {
                        id = id.saturating_add(1);
                        id_buffer = id.to_string();
                        is_dirty = true;
                    }

                    ui.add_space(20.0);
                    ui.label(format!("Filter: {:?}", filter));

                    if original_size[0] > 0 {
                        ui.add_space(20.0);
                        ui.label(format!(
                            "Original: {}x{}",
                            original_size[0], original_size[1]
                        ));
                        ui.label(format!(
                            "Upscaled: {}x{}",
                            upscaled_size[0], upscaled_size[1]
                        ));
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Reset").clicked() {
                            zoom = 1.0;
                        }
                        ui.add(egui::Slider::new(&mut zoom, 0.1..=20.0).logarithmic(true));
                        if ui.button("➕").clicked() {
                            zoom = (zoom * 1.2).min(20.0);
                        }
                        if ui.button("➖").clicked() {
                            zoom = (zoom / 1.2).max(0.1);
                        }
                        ui.label(format!("{:.1}x", zoom));
                    });
                });
                ui.separator();

                // Handle zoom shortcuts and mouse wheel
                if ui.ui_contains_pointer() || ui.is_enabled() {
                    let zoom_delta = ui.input(|i| i.smooth_scroll_delta.y);
                    if zoom_delta != 0.0 && ui.input(|i| i.modifiers.command) {
                        zoom = (zoom * (zoom_delta * 0.005).exp()).clamp(0.1, 20.0);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Plus)) {
                         zoom = (zoom * 1.2).min(20.0);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Minus)) {
                         zoom = (zoom / 1.2).max(0.1);
                    }
                }

                egui::ScrollArea::both().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if let Some(tex) = &texture {
                            ui.vertical(|ui| {
                                ui.label("Original");
                                let sz = egui::vec2(
                                    original_size[0] as f32 * zoom,
                                    original_size[1] as f32 * zoom,
                                );
                                ui.add(egui::Image::new(tex).fit_to_exact_size(sz));
                            });
                        }
                        ui.add_space(20.0);
                        if let Some(tex) = &upscaled_texture {
                            ui.vertical(|ui| {
                                ui.label("Upscaled");
                                let sz = egui::vec2(
                                    upscaled_size[0] as f32 * zoom,
                                    upscaled_size[1] as f32 * zoom,
                                );
                                ui.add(egui::Image::new(tex).fit_to_exact_size(sz));
                            });
                        }
                    });
                });
            });

        // Sync back
        if let Some(preview) = self.upscale_preview.as_mut() {
            preview.id = id;
            preview.id_buffer = id_buffer;
            preview.zoom = zoom;
            if is_dirty {
                preview.is_dirty = true;
            }
        }

        if !open {
            self.upscale_preview = None;
        }
    }
}
