//! View of the Classic fonts (`fonts.mul`, `unifont*.mul`) through `uocf::classic::fonts`: the glyphs of an ASCII font,
//! or a 256-character page of a Unicode font and a sample text. `Edit` is in [`super::classic_fonts_editor`].

use eframe::egui;
use uocf::classic::fonts::{ASCII_GLYPH_COUNT, ASCII_PRINTABLE_START, ClassicFonts, FontRgbaImage};

use super::classic_fonts_editor::FontsEditor;
use super::{arrow_delta, move_selection};
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
const GO_TO_WIDTH: f32 = 90.0;
const GO_TO_HINT: &str = "A, U+00E9, 233";
const GO_TO_HOVER: &str = "A character, its code point (U+00E9 or 0xE9) or its decimal code (233), then Enter. A single \
                           digit is the character itself.";
const FIRST_ASCII_CODE: u32 = ASCII_PRINTABLE_START as u32;
const LAST_ASCII_CODE: u32 = FIRST_ASCII_CODE + ASCII_GLYPH_COUNT as u32 - 1;
const LAST_CODE_POINT: u32 = u16::MAX as u32;
const HEX_RADIX: u32 = 16;

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
    /// What "Go to" was given.
    go_to: String,
    /// The glyph "Go to" found, outlined while viewing, and whether the grid still has to scroll to it.
    found: Option<u32>,
    scroll_to_found: bool,
    /// Why the last "Go to" found nothing.
    go_to_note: Option<String>,
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
            go_to: String::new(),
            found: None,
            scroll_to_found: false,
            go_to_note: None,
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

/// The code "Go to" was given: `U+00E9` or `0xE9` (hexadecimal), a single character (itself, digits too), or a
/// decimal code.
fn parse_code(text: &str) -> Option<u32> {
    let text = text.trim();
    let hex = ["U+", "u+", "0x", "0X"].iter().find_map(|prefix| text.strip_prefix(prefix));
    if let Some(hex) = hex {
        return u32::from_str_radix(hex, HEX_RADIX).ok();
    }
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(single), None) => Some(single as u32),
        _ => text.parse().ok(),
    }
}

/// Shows the glyph `state.go_to` names in the chosen font: turns to its Unicode page, selects it for editing and
/// scrolls the grid to it, or says why it cannot.
fn go_to(state: &mut ClassicFontsState) {
    let Some(Ok(fonts)) = &state.fonts else {
        return;
    };
    state.go_to_note = None;
    let Some(code) = parse_code(&state.go_to) else {
        state.go_to_note = Some(format!("\"{}\" is not a character or a code.", state.go_to.trim()));
        return;
    };
    let present = match state.choice {
        FontChoice::Ascii(_) => (FIRST_ASCII_CODE..=LAST_ASCII_CODE).contains(&code),
        FontChoice::Unicode(font) => {
            code <= LAST_CODE_POINT
                && match state.editor.unicode_file(font) {
                    Some(file) => file.glyph(code as u16).is_some(),
                    None => matches!(fonts.unicode_glyph(font, code as u16), Ok(Some(_))),
                }
        }
    };
    if !present {
        state.go_to_note = Some(format!("{} U+{code:04X} has no glyph in this font.", char_label(code)));
        return;
    }
    if let FontChoice::Unicode(_) = state.choice {
        state.page = code / PAGE_GLYPHS;
    }
    state.editor.select(code);
    state.found = Some(code);
    state.scroll_to_found = true;
}

/// The fonts of the list, in its order.
fn font_choices(fonts: &ClassicFonts) -> Vec<FontChoice> {
    (0..fonts.ascii_font_count())
        .map(FontChoice::Ascii)
        .chain((0..UNICODE_FONT_SLOTS).filter(|&font| fonts.unicode_font_exists(font)).map(FontChoice::Unicode))
        .collect()
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
        let choices = font_choices(fonts);
        // The arrow keys browse the list, unless a text field has the keyboard.
        let keyboard_moved = match arrow_delta(ui, ui.ctx().text_edit_focused()).and_then(|delta| move_selection(&choices, Some(state.choice), delta)) {
            Some(choice) if choice != state.choice => {
                state.choice = choice;
                true
            }
            _ => false,
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            for &choice in &choices {
                let response = ui.selectable_value(&mut state.choice, choice, choice_label(choice));
                if keyboard_moved && state.choice == choice {
                    response.scroll_to_me(Some(egui::Align::Center));
                }
            }
        });
    });

    egui::CentralPanel::default().show(ui, |ui| {
        if !matches!(state.fonts, Some(Ok(_))) {
            return;
        }
        ui.heading(choice_label(state.choice));
        ui.horizontal(|ui| {
            ui.add(egui::Slider::new(&mut state.zoom, MIN_ZOOM..=MAX_ZOOM).text("Zoom"));
            if let FontChoice::Unicode(_) = state.choice {
                ui.add(egui::DragValue::new(&mut state.page).range(0..=LAST_PAGE).prefix("page ").custom_formatter(|page, _| format!("{:04X}", page as u32 * PAGE_GLYPHS)));
            }
            ui.label("Go to:");
            let field = ui.add(egui::TextEdit::singleline(&mut state.go_to).hint_text(GO_TO_HINT).desired_width(GO_TO_WIDTH)).on_hover_text(GO_TO_HOVER);
            if field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                go_to(state);
            }
            if let Some(note) = &state.go_to_note {
                ui.colored_label(ui.visuals().warn_fg_color, note);
            }
        });
        let Some(Ok(fonts)) = &state.fonts else { return };
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
        let outlined = if editing { Some(state.editor.selected()) } else { state.found };
        let grid = egui::ScrollArea::vertical().show(ui, |ui| {
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
                    if outlined == Some(glyph.code) {
                        let stroke = egui::Stroke::new(SELECTED_STROKE_WIDTH, ui.visuals().selection.bg_fill);
                        ui.painter().rect_stroke(response.rect, 0.0, stroke, egui::StrokeKind::Inside);
                        if state.scroll_to_found {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                    }
                    if editing {
                        let response = response.interact(egui::Sense::click());
                        if response.clicked() {
                            state.editor.select(glyph.code);
                        }
                        response.on_hover_text(&glyph.details);
                    } else {
                        response.on_hover_text(&glyph.details);
                    }
                }
            });
        });
        // "Go to" turns the page and fills the grid in the same frame.
        state.scroll_to_found = false;
        // Ctrl or Shift + wheel, or a pinch, over the glyphs zooms them.
        let zoom = super::hovered_zoom_factor(ui, grid.inner_rect);
        if zoom != super::NO_ZOOM {
            state.zoom = (state.zoom * zoom).clamp(MIN_ZOOM, MAX_ZOOM);
        }
    });
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use uocf::classic::fonts::{unicode_font_file_name, AsciiFont, AsciiFontGlyph, AsciiFontsFile, UnicodeFontData, UnicodeFontGlyph};

    use super::*;
    use crate::app::tests::test_app;
    use crate::ui::test_support::output_path;

    const ASCII_FONTS: usize = 2;
    const GLYPH_SIDE: u8 = 2;
    /// 'é', on the first page.
    const NEAR_CODE: u16 = 0x00E9;
    /// On a later page.
    const FAR_CODE: u16 = 0x4E00;
    const ABSENT_CODE: u16 = 0x4E01;

    /// A client folder with two ASCII fonts and one Unicode font, which the loader also lists as font 1.
    fn client_folder(name: &str) -> PathBuf {
        let folder = output_path(name);
        std::fs::create_dir_all(&folder).unwrap();
        let pixels = vec![0; usize::from(GLYPH_SIDE) * usize::from(GLYPH_SIDE)];
        let glyph = AsciiFontGlyph { width: GLYPH_SIDE, height: GLYPH_SIDE, unknown: 0, pixels };
        let font = AsciiFont { header: 0, glyphs: vec![glyph; ASCII_GLYPH_COUNT] };
        AsciiFontsFile { fonts: vec![font; ASCII_FONTS], tail: Vec::new() }.save(&folder.join("fonts.mul")).unwrap();
        let mut unicode = UnicodeFontData::default();
        for code in [NEAR_CODE, FAR_CODE] {
            unicode.set_glyph(code, Some(UnicodeFontGlyph::blank(GLYPH_SIDE, GLYPH_SIDE))).unwrap();
        }
        unicode.save(&folder.join(unicode_font_file_name(0))).unwrap();
        folder
    }

    fn fonts_app(folder: &Path) -> UopInspectorApp {
        let mut app = test_app();
        app.view_mode = crate::app::ViewMode::ClassicFonts;
        app.settings.cc_path = Some(folder.to_path_buf());
        app
    }

    fn frame(ctx: &egui::Context, app: &mut UopInspectorApp, input: egui::RawInput) {
        let _ = ctx.run_ui(input, |ui| crate::ui::draw_ui(app, ctx, ui));
    }

    fn key_press(key: egui::Key) -> egui::RawInput {
        let event = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        egui::RawInput { events: vec![event], ..Default::default() }
    }

    #[test]
    fn codes_are_read_as_a_character_a_code_point_or_a_decimal_code() {
        assert_eq!(parse_code("A"), Some(u32::from('A')));
        assert_eq!(parse_code("5"), Some(u32::from('5')), "a single digit is the character");
        assert_eq!(parse_code("é"), Some(0xE9));
        assert_eq!(parse_code(" U+00E9 "), Some(0xE9));
        assert_eq!(parse_code("0x4e00"), Some(0x4E00));
        assert_eq!(parse_code("233"), Some(233));
        assert_eq!(parse_code("U+"), None);
        assert_eq!(parse_code("abc"), None);
    }

    #[test]
    fn the_arrow_keys_browse_the_font_list() {
        let folder = client_folder("fonts-arrows");
        let mut app = fonts_app(&folder);
        let ctx = egui::Context::default();
        frame(&ctx, &mut app, egui::RawInput::default());
        assert_eq!(app.classic_file_views.fonts.choice, FontChoice::Ascii(0));
        let expected = [FontChoice::Ascii(1), FontChoice::Unicode(0), FontChoice::Unicode(1), FontChoice::Unicode(1)];
        for choice in expected {
            frame(&ctx, &mut app, key_press(egui::Key::ArrowDown));
            assert_eq!(app.classic_file_views.fonts.choice, choice);
        }
        frame(&ctx, &mut app, key_press(egui::Key::ArrowUp));
        assert_eq!(app.classic_file_views.fonts.choice, FontChoice::Unicode(0));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn go_to_turns_to_the_page_of_a_glyph_and_selects_it() {
        let folder = client_folder("fonts-go-to");
        let fonts = ClassicFonts::load(&folder).unwrap();
        let mut state = ClassicFontsState { fonts: Some(Ok(fonts)), choice: FontChoice::Unicode(0), ..Default::default() };
        state.go_to = format!("U+{FAR_CODE:04X}");
        go_to(&mut state);
        assert_eq!((state.page, state.found, state.editor.selected()), (u32::from(FAR_CODE) / PAGE_GLYPHS, Some(u32::from(FAR_CODE)), u32::from(FAR_CODE)));
        assert!(state.scroll_to_found && state.go_to_note.is_none());

        state.go_to = format!("U+{ABSENT_CODE:04X}");
        go_to(&mut state);
        assert!(state.go_to_note.is_some(), "no glyph");
        assert_eq!(state.found, Some(u32::from(FAR_CODE)), "kept");

        state.choice = FontChoice::Ascii(0);
        state.go_to = "A".to_owned();
        go_to(&mut state);
        assert_eq!(state.found, Some(u32::from('A')));
        state.go_to = "é".to_owned();
        go_to(&mut state);
        assert_eq!(state.found, Some(u32::from('é')), "ASCII fonts end at 255");
        state.go_to = "U+0100".to_owned();
        go_to(&mut state);
        assert!(state.go_to_note.is_some(), "past the ASCII font");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
