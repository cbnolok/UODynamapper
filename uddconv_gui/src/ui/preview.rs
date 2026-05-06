use eframe::egui;
use crate::app::UddConvApp;

impl UddConvApp {
    pub fn ui_preview_window(&mut self, ctx: &egui::Context) {
        let Some(path) = self.preview_path.clone() else {
            return;
        };

        let mut open = true;
        egui::Window::new(format!(
            "Preview: {}",
            path.file_name().unwrap().to_string_lossy()
        ))
        .open(&mut open)
        .resizable(true)
        .default_size([800.0, 800.0])
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

        if path.extension().map_or(false, |ext| ext == "ktx2") {
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
        let extent = uddconv::bc7::ImageExtent::new(width, height).ok()?;
        let rgba = uddconv::bc7::decode_bc7_to_rgba8888(&blocks, extent).ok()?;

        Some(egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            &rgba,
        ))
    }
}
