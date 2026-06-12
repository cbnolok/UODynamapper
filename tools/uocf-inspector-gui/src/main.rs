mod app;
mod dialog;
mod logic;
mod ui;

use app::UopInspectorApp;
use eframe::egui;
use std::sync::Arc;

const APP_VIEWPORT_WIDTH: f32 = 1200.0;
const APP_VIEWPORT_HEIGHT: f32 = 800.0;
const APP_MIN_VIEWPORT_WIDTH: f32 = 400.0;
const APP_MIN_VIEWPORT_HEIGHT: f32 = 300.0;

fn main() -> eframe::Result {
    dialog::normalize_linux_portal_env_before_threads();
    let _ = udd_logging::install_paris_logger();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([APP_VIEWPORT_WIDTH, APP_VIEWPORT_HEIGHT])
            .with_min_inner_size([APP_MIN_VIEWPORT_WIDTH, APP_MIN_VIEWPORT_HEIGHT]),
        ..Default::default()
    };

    eframe::run_native(
        "UOCF Inspector",
        native_options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            install_system_unicode_font_fallback(&cc.egui_ctx);
            Ok(Box::new(UopInspectorApp::new(cc)))
        }),
    )
}

fn install_system_unicode_font_fallback(ctx: &egui::Context) {
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
