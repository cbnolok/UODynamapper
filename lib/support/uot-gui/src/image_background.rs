//! The background an image viewer draws behind its image, so halos and stray texels at the edges of the art show
//! against a colour the user picks: egui's HSV colour picker, or a preset.

use eframe::egui;
use serde::{Deserialize, Serialize};

/// What an image viewer draws behind its image. The apps keep it in their settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageBackground {
    /// Nothing: the panel or window under the image shows, as each viewer did before it had the option.
    #[default]
    ViewerDefault,
    /// An opaque colour, as sRGB bytes.
    Color([u8; 3]),
}

impl ImageBackground {
    /// The colour drawn behind the image, `None` for the viewer's default.
    pub fn color(self) -> Option<egui::Color32> {
        match self {
            Self::ViewerDefault => None,
            Self::Color([red, green, blue]) => Some(egui::Color32::from_rgb(red, green, blue)),
        }
    }

    /// `image` with this background filled under it, over the image's rect.
    pub fn under(self, image: egui::Image<'_>) -> egui::Image<'_> {
        match self.color() {
            Some(color) => image.bg_fill(color),
            None => image,
        }
    }
}

pub const BLACK_BACKGROUND: [u8; 3] = [0, 0, 0];
pub const WHITE_BACKGROUND: [u8; 3] = [u8::MAX, u8::MAX, u8::MAX];
/// sRGB 128, half of each channel's range.
pub const MID_GREY_BACKGROUND: [u8; 3] = [128, 128, 128];
/// A colour no art uses, against which any stray texel shows.
pub const MAGENTA_BACKGROUND: [u8; 3] = [u8::MAX, 0, u8::MAX];

/// The presets shown beside the colour picker, in order, with their labels.
pub const IMAGE_BACKGROUND_PRESETS: [(&str, ImageBackground); 5] = [
    ("Default", ImageBackground::ViewerDefault),
    ("Black", ImageBackground::Color(BLACK_BACKGROUND)),
    ("White", ImageBackground::Color(WHITE_BACKGROUND)),
    ("Grey", ImageBackground::Color(MID_GREY_BACKGROUND)),
    ("Magenta", ImageBackground::Color(MAGENTA_BACKGROUND)),
];

const PICKER_HOVER: &str = "The colour behind the image, so halos and stray pixels at the edges of the art show. Click \
    to pick one (hue, saturation and value).";
const DEFAULT_HOVER: &str = "What this viewer showed before: the colour of the panel under the image.";

/// The background control: a colour button opening egui's HSV picker, then the presets. While the background is the
/// viewer's default, the button shows `default_fill`, the colour that shows under the image then. Returns whether the
/// user changed `background`.
pub fn image_background_control(ui: &mut egui::Ui, background: &mut ImageBackground, default_fill: egui::Color32) -> bool {
    let mut changed = false;
    ui.label("Background:");
    let mut rgb = match *background {
        ImageBackground::ViewerDefault => [default_fill.r(), default_fill.g(), default_fill.b()],
        ImageBackground::Color(rgb) => rgb,
    };
    if ui.color_edit_button_srgb(&mut rgb).on_hover_text(PICKER_HOVER).changed() {
        *background = ImageBackground::Color(rgb);
        changed = true;
    }
    for (label, preset) in IMAGE_BACKGROUND_PRESETS {
        let mut button = ui.add(egui::Button::selectable(*background == preset, label));
        if preset == ImageBackground::ViewerDefault {
            button = button.on_hover_text(DEFAULT_HOVER);
        }
        if button.clicked() && *background != preset {
            *background = preset;
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE_SIZE: egui::Vec2 = egui::vec2(16.0, 8.0);

    /// The rect shapes one pass paints over the rect of an image under `background`, as (fill, textured).
    fn painted(background: ImageBackground) -> Vec<(egui::Color32, bool)> {
        let ctx = egui::Context::default();
        let mut rect = egui::Rect::NOTHING;
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            rect = ui.add(background.under(egui::Image::new((egui::TextureId::default(), IMAGE_SIZE)))).rect;
        });
        assert_eq!(rect.size(), IMAGE_SIZE);
        output
            .shapes
            .into_iter()
            .filter_map(|clipped| match clipped.shape {
                egui::Shape::Rect(shape) if shape.rect == rect => Some((shape.fill, shape.brush.is_some())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_colour_is_filled_under_the_image_over_its_rect() {
        let shapes = painted(ImageBackground::Color(MAGENTA_BACKGROUND));
        assert_eq!(shapes, [(egui::Color32::MAGENTA, false), (egui::Color32::WHITE, true)]);
    }

    #[test]
    fn the_viewer_default_fills_nothing() {
        assert_eq!(painted(ImageBackground::ViewerDefault), [(egui::Color32::WHITE, true)]);
    }

    #[test]
    fn the_presets_start_with_the_viewer_default_and_differ() {
        assert_eq!(ImageBackground::default(), IMAGE_BACKGROUND_PRESETS[0].1);
        for (index, (_, preset)) in IMAGE_BACKGROUND_PRESETS.iter().enumerate() {
            assert!(IMAGE_BACKGROUND_PRESETS[index + 1..].iter().all(|(_, other)| other != preset));
        }
    }
}
