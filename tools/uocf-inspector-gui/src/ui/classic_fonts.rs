//! View of the Classic fonts (`fonts.mul`, `unifont*.mul`) through `uocf::classic::fonts`: the glyphs of an ASCII font,
//! or a 256-character page of a Unicode font and a sample text. `Edit` is in [`super::classic_fonts_editor`].

use eframe::egui;
use uocf::classic::fonts::{ASCII_GLYPH_COUNT, ASCII_PRINTABLE_START, ClassicFonts, FontRgbaImage};

use super::classic_fonts_editor::FontsEditor;
use crate::app::UopInspectorApp;

/// `ClassicFonts` looks for `unifont.mul` to `unifont19.mul`.
const UNICODE_FONT_SLOTS: usize = 20;
const PAGE_GLYPHS: u32 = 256;
const LAST_PAGE: u32 = (u16::MAX as u32 + 1) / PAGE_GLYPHS - 1;
pub(super) const FOREGROUND: [u8; 4] = [u8::MAX; 4];
const MIN_ZOOM: f32 = 1.0;
const MAX_ZOOM: f32 = 8.0;
const DEFAULT_ZOOM: f32 = 2.0;
const CELL_PADDING: f32 = 6.0;
/// Wide enough for the label below a narrow glyph.
const CELL_MIN_WIDTH: f32 = 44.0;
const CELL_LABEL_HEIGHT: f32 = 16.0;
/// What the frame of a cell adds to its content, on both sides.
const CELL_FRAME: f32 = 14.0;
const DEFAULT_SAMPLE: &str = "The quick brown fox jumps over the lazy dog 0123456789";
const TOOLS_PANEL_WIDTH: f32 = 220.0;
const SELECTED_STROKE_WIDTH: f32 = 2.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FontChoice {
    Ascii(usize),
    Unicode(usize),
}

struct Glyph {
    /// Its character or code point.
    code: u32,
    label: String,
    details: String,
    texture: Option<egui::TextureHandle>,
    size: egui::Vec2,
}

pub struct ClassicFontsState {
    fonts: Option<Result<ClassicFonts, String>>,
    choice: FontChoice,
    page: u32,
    zoom: f32,
    sample: String,
    /// The glyphs of `built_for`.
    glyphs: Vec<Glyph>,
    sample_texture: Option<(String, egui::TextureHandle)>,
    /// The font, the page and the editor's generation.
    built_for: Option<(FontChoice, u32, u64)>,
    pub(super) editor: FontsEditor,
}

impl Default for ClassicFontsState {
    fn default() -> Self {
        Self {
            fonts: None,
            choice: FontChoice::Ascii(0),
            page: 0,
            zoom: DEFAULT_ZOOM,
            sample: DEFAULT_SAMPLE.to_owned(),
            glyphs: Vec::new(),
            sample_texture: None,
            built_for: None,
            editor: FontsEditor::default(),
        }
    }
}

fn texture(ctx: &egui::Context, name: String, image: &FontRgbaImage) -> Option<egui::TextureHandle> {
    super::textures::load_rgba(ctx, name, [image.width as usize, image.height as usize], &image.rgba, egui::TextureOptions::NEAREST).ok()
}

fn char_label(codepoint: u32) -> String {
    char::from_u32(codepoint).filter(|c| !c.is_control()).map_or_else(|| format!("U+{codepoint:04X}"), |c| c.to_string())
}

/// The glyphs of the file being edited when it is open, else of the loaded one.
fn build_glyphs(ctx: &egui::Context, fonts: &ClassicFonts, editor: &FontsEditor, choice: FontChoice, page: u32) -> Vec<Glyph> {
    let mut glyphs = Vec::new();
    match choice {
        FontChoice::Ascii(font) => {
            let Some(font_data) = editor.ascii_font(font).or_else(|| fonts.ascii_font(font)) else { return glyphs };
            for (index, glyph) in font_data.glyphs.iter().enumerate().take(ASCII_GLYPH_COUNT) {
                let code = u32::from(ASCII_PRINTABLE_START) + index as u32;
                let (texture, details) = match glyph.to_rgba8() {
                    Ok(image) => (texture(ctx, format!("ascii_font_{font}_{index}"), &image), format!("{} x {}, byte 3 = {}", glyph.width, glyph.height, glyph.unknown)),
                    Err(error) => (None, format!("{error:#}")),
                };
                glyphs.push(Glyph { code, label: format!("{} ({code})", char_label(code)), details, texture, size: egui::vec2(glyph.width.into(), glyph.height.into()) });
            }
        }
        FontChoice::Unicode(font) => {
            for code in page * PAGE_GLYPHS..(page + 1) * PAGE_GLYPHS {
                let glyph = match editor.unicode_file(font) {
                    Some(file) => Ok(file.glyph(code as u16).cloned()),
                    None => fonts.unicode_glyph(font, code as u16),
                };
                let (texture, details, size) = match glyph {
                    Ok(Some(glyph)) => match glyph.to_rgba8(FOREGROUND, None) {
                        Ok(image) => (
                            texture(ctx, format!("unicode_font_{font}_{code}"), &image),
                            format!("{} x {}, offset {}, {}", glyph.width, glyph.height, glyph.offset_x, glyph.offset_y),
                            egui::vec2(glyph.width.into(), glyph.height.into()),
                        ),
                        Err(error) => (None, format!("{error:#}"), egui::Vec2::ZERO),
                    },
                    Ok(None) => continue,
                    Err(error) => (None, format!("{error:#}"), egui::Vec2::ZERO),
                };
                glyphs.push(Glyph { code, label: format!("{} U+{code:04X}", char_label(code)), details, texture, size });
            }
        }
    }
    glyphs
}

fn choice_label(choice: FontChoice) -> String {
    match choice {
        FontChoice::Ascii(font) => format!("ASCII font {font} (fonts.mul)"),
        FontChoice::Unicode(0) => "Unicode font 0 (unifont.mul)".to_owned(),
        FontChoice::Unicode(font) => format!("Unicode font {font} (unifont{font}.mul)"),
    }
}

pub fn ui_classic_fonts(app: &mut UopInspectorApp, ctx: &egui::Context, ui: &mut egui::Ui) {
    let choice = app.classic_file_views.fonts.choice;
    egui::Panel::top("classic_fonts_editor_bar").show(ui, |ui| {
        ui.horizontal_wrapped(|ui| super::classic_fonts_editor::bar(app, ui, choice));
    });
    let cc_path = app.settings.cc_path.clone();
    let state = &mut app.classic_file_views.fonts;
    egui::Panel::left("classic_fonts_tools").resizable(true).default_size(TOOLS_PANEL_WIDTH).show(ui, |ui| {
        ui.heading("Fonts");
        ui.separator();
        let Some(cc_path) = cc_path else {
            ui.label("Set the CC path (File > Search Paths).");
            return;
        };
        if state.fonts.is_none() || ui.button("Read again").clicked() {
            state.fonts = Some(ClassicFonts::load(&cc_path).map_err(|error| format!("{error:#}")));
            state.built_for = None;
            state.sample_texture = None;
            state.editor.forget_unchanged();
        }
        let fonts = match &state.fonts {
            Some(Ok(fonts)) => fonts,
            Some(Err(error)) => {
                ui.colored_label(ui.visuals().error_fg_color, error);
                return;
            }
            None => return,
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            for font in 0..fonts.ascii_font_count() {
                ui.selectable_value(&mut state.choice, FontChoice::Ascii(font), choice_label(FontChoice::Ascii(font)));
            }
            for font in (0..UNICODE_FONT_SLOTS).filter(|&font| fonts.unicode_font_exists(font)) {
                ui.selectable_value(&mut state.choice, FontChoice::Unicode(font), choice_label(FontChoice::Unicode(font)));
            }
        });
    });

    egui::CentralPanel::default().show(ui, |ui| {
        let Some(Ok(fonts)) = &state.fonts else { return };
        ui.heading(choice_label(state.choice));
        ui.horizontal(|ui| {
            ui.add(egui::Slider::new(&mut state.zoom, MIN_ZOOM..=MAX_ZOOM).text("Zoom"));
            if let FontChoice::Unicode(_) = state.choice {
                ui.add(egui::DragValue::new(&mut state.page).range(0..=LAST_PAGE).prefix("page ").custom_formatter(|page, _| format!("{:04X}", page as u32 * PAGE_GLYPHS)));
            }
        });
        if let FontChoice::Unicode(font) = state.choice {
            ui.horizontal(|ui| {
                ui.label("Sample:");
                ui.text_edit_singleline(&mut state.sample);
            });
            let key = format!("{font}:{}", state.sample);
            if state.sample_texture.as_ref().is_none_or(|(built, _)| *built != key) {
                state.sample_texture = fonts
                    .unicode_text_rgba8(font, &state.sample, FOREGROUND, None)
                    .ok()
                    .and_then(|image| texture(ctx, format!("unicode_sample_{font}"), &image))
                    .map(|texture| (key, texture));
            }
            if let Some((_, texture)) = &state.sample_texture {
                ui.add(egui::Image::new(texture).fit_to_exact_size(texture.size_vec2() * state.zoom));
            }
            ui.separator();
        }
        let editing = state.editor.enabled;
        if editing {
            state.editor.glyph_panel(ui, state.choice);
            ui.separator();
        }
        let wanted = (state.choice, state.page, state.editor.generation());
        if state.built_for != Some(wanted) {
            state.glyphs = build_glyphs(ctx, fonts, &state.editor, state.choice, state.page);
            state.built_for = Some(wanted);
        }
        if state.glyphs.is_empty() {
            ui.weak("No glyph on this page.");
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let tallest = state.glyphs.iter().map(|glyph| glyph.size.y).fold(0.0, f32::max);
                for glyph in &state.glyphs {
                    // A frame does not wrap by itself: the wrapping layout has to be told its size first.
                    let width = (glyph.size.x * state.zoom + CELL_PADDING).max(CELL_MIN_WIDTH);
                    let cell = egui::vec2(width + CELL_FRAME, tallest * state.zoom + CELL_LABEL_HEIGHT + CELL_FRAME);
                    let response = ui
                        .allocate_ui(cell, |ui| {
                            ui.group(|ui| {
                                ui.set_min_width(width);
                                ui.vertical_centered(|ui| {
                                    match &glyph.texture {
                                        Some(texture) => ui.add(egui::Image::new(texture).fit_to_exact_size(glyph.size * state.zoom)),
                                        None => ui.weak("-"),
                                    };
                                    ui.small(&glyph.label);
                                });
                            })
                            .response
                        })
                        .inner;
                    if editing {
                        let response = response.interact(egui::Sense::click());
                        if response.clicked() {
                            state.editor.select(glyph.code);
                        }
                        if state.editor.selected() == glyph.code {
                            let stroke = egui::Stroke::new(SELECTED_STROKE_WIDTH, ui.visuals().selection.bg_fill);
                            ui.painter().rect_stroke(response.rect, 0.0, stroke, egui::StrokeKind::Inside);
                        }
                        response.on_hover_text(&glyph.details);
                    } else {
                        response.on_hover_text(&glyph.details);
                    }
                }
            });
        });
    });
}
