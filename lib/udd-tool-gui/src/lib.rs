use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;

pub type AppCreatorResult =
    Result<Box<dyn eframe::App>, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Copy, Debug)]
pub struct NativeWindow {
    pub title: &'static str,
    pub width: f32,
    pub height: f32,
    pub min_width: f32,
    pub min_height: f32,
}

pub fn native_options(window: NativeWindow) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([window.width, window.height])
            .with_min_inner_size([window.min_width, window.min_height])
            .with_title(window.title),
        ..Default::default()
    }
}

pub fn run_native_with_setup<F>(
    app_id: &str,
    window: NativeWindow,
    install_unicode_font_fallback: bool,
    app_creator: F,
) -> eframe::Result
where
    F: FnOnce(&eframe::CreationContext<'_>) -> AppCreatorResult + 'static,
{
    normalize_linux_portal_env_before_threads();

    eframe::run_native(
        app_id,
        native_options(window),
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            if install_unicode_font_fallback {
                install_system_unicode_font_fallback(&cc.egui_ctx);
            }
            app_creator(cc)
        }),
    )
}

pub fn file_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new()
}

pub fn pick_folder() -> Option<PathBuf> {
    file_dialog().pick_folder()
}

pub fn open_filtered_file_dialog(
    initial_dir: Option<&Path>,
    filter_name: &str,
    extensions: &[&str],
) -> Option<PathBuf> {
    let mut dialog = file_dialog().add_filter(filter_name, extensions);
    if let Some(initial_dir) = initial_dir.filter(|path| path.is_dir()) {
        dialog = dialog.set_directory(initial_dir);
    }
    dialog.pick_file()
}

pub fn save_file_dialog(default_name: &str) -> Option<PathBuf> {
    file_dialog().set_file_name(default_name).save_file()
}

pub fn install_system_unicode_font_fallback(ctx: &egui::Context) {
    let Some((font_name, font_bytes)) = system_unicode_font_candidates()
        .iter()
        .find_map(|path| std::fs::read(path).ok().map(|bytes| ((*path).to_string(), bytes)))
    else {
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        font_name.clone(),
        Arc::new(egui::FontData::from_owned(font_bytes)),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .push(font_name.clone());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .push(font_name);
    ctx.set_fonts(fonts);
}

fn system_unicode_font_candidates() -> &'static [&'static str] {
    &[
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
        "/usr/share/fonts/opentype/source-han-sans/SourceHanSans-Regular.ttc",
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\simsun.ttc",
        "C:\\Windows\\Fonts\\msgothic.ttc",
        "C:\\Windows\\Fonts\\malgun.ttf",
    ]
}

#[cfg(target_os = "linux")]
pub fn normalize_linux_portal_env_before_threads() {
    use std::env;

    let current = env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if !current.is_empty() {
        return;
    }

    let session = env::var("XDG_SESSION_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let desktop_session = env::var("DESKTOP_SESSION")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let kde_full = env::var("KDE_FULL_SESSION").unwrap_or_default();

    let inferred = if kde_full.eq_ignore_ascii_case("true")
        || session.contains("kde")
        || session.contains("plasma")
        || desktop_session.contains("kde")
        || desktop_session.contains("plasma")
    {
        Some("KDE")
    } else if session.contains("gnome") || desktop_session.contains("gnome") {
        Some("GNOME")
    } else {
        None
    };

    if let Some(desktop) = inferred {
        unsafe {
            // SAFETY: callers run this during GUI startup before eframe spawns worker threads.
            env::set_var("XDG_CURRENT_DESKTOP", desktop);
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn normalize_linux_portal_env_before_threads() {}
