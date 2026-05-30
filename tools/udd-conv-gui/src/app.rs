use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use eframe::egui;
use crate::models::{AppSettings, LogMessage, LogLevel, Tab};
use crate::logic::settings::{load_settings_report, save_settings};

pub struct UddConvApp {
    pub settings: AppSettings,
    pub logs: Arc<Mutex<Vec<LogMessage>>>,
    pub is_converting: Arc<Mutex<bool>>,
    pub current_tab: Tab,

    // Tool state
    pub tool_file_1: Option<PathBuf>,
    pub tool_file_2: Option<PathBuf>,
    pub preview_path: Option<PathBuf>,
    pub preview_texture: Option<egui::TextureHandle>,
    pub upscale_preview: Option<crate::models::UpscalePreviewState>,
}

impl UddConvApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (settings, settings_warning) = load_settings_report();
        let logs = Arc::new(Mutex::new(Vec::new()));
        crate::logic::panel_logger::install_panel_logger(logs.clone());

        {
            let mut logs_guard = logs.lock().expect("initialize GUI logs");
            logs_guard.push(LogMessage {
                text: "Application started. Please configure your source directories.".to_string(),
                level: LogLevel::Info,
            });
            if let Some(warning) = settings_warning {
                logs_guard.push(LogMessage {
                    text: warning,
                    level: LogLevel::Error,
                });
            }
        }

        Self {
            settings,
            logs,
            is_converting: Arc::new(Mutex::new(false)),
            current_tab: Tab::Sources,
            tool_file_1: None,
            tool_file_2: None,
            preview_path: None,
            preview_texture: None,
            upscale_preview: None,
        }
    }

    pub fn push_log(&self, text: impl Into<String>, level: LogLevel) {
        if let Ok(mut logs) = self.logs.lock() {
            logs.push(LogMessage {
                text: text.into(),
                level,
            });
        }
    }

    pub fn is_busy(&self) -> bool {
        self.is_converting.lock().map(|busy| *busy).unwrap_or(false)
    }

    pub fn persist_settings(&self) {
        if let Err(error) = save_settings(&self.settings) {
            self.push_log(format!("Could not save settings: {}", error), LogLevel::Error);
        }
    }

    pub fn get_output_path(&self, filename: &str) -> PathBuf {
        self.settings.output_uddp_dir.join(filename)
    }

    pub fn get_input_uddp_path(&self, filename: &str) -> PathBuf {
        self.settings.input_uddp_dir.join(filename)
    }

    pub fn open_upscale_preview(&mut self, target: crate::models::UpscalePreviewTarget, filter: udd_conv::upscale::UpscaleFilter) {
        self.upscale_preview = Some(crate::models::UpscalePreviewState {
            target,
            id: 0,
            filter,
            texture: None,
            upscaled_texture: None,
            upscaled_size: [0, 0],
            original_size: [0, 0],
            id_buffer: "0".to_string(),
            is_dirty: true,
            zoom: 1.0,
        });
    }

    pub fn update_upscale_preview(&mut self, ctx: &egui::Context) {
        let Some(preview) = &mut self.upscale_preview else {
            return;
        };
        if !preview.is_dirty {
            return;
        }

        let cc_dir = self.settings.cc_dir.as_deref();
        let ec_dir = self.settings.ec_dir.as_deref();
        let mut preview_error = None;

        match crate::logic::preview::load_raw_asset(cc_dir, ec_dir, preview.target, preview.id) {
            Ok(raw) => {
                preview.original_size = [raw.width, raw.height];

                // Original texture
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [raw.width as usize, raw.height as usize],
                    &raw.rgba,
                );
                preview.texture =
                    Some(ctx.load_texture("preview_orig", color_image, Default::default()));

                // Upscaled texture
                let (up_w, up_h, up_rgba) = preview.filter.apply(raw.width, raw.height, &raw.rgba);
                preview.upscaled_size = [up_w, up_h];
                let up_color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [up_w as usize, up_h as usize],
                    &up_rgba,
                );
                preview.upscaled_texture = Some(ctx.load_texture(
                    "preview_upscaled",
                    up_color_image,
                    Default::default(),
                ));
            }
            Err(e) => {
                preview.texture = None;
                preview.upscaled_texture = None;
                preview_error = Some(format!("Preview error: {}", e));
            }
        }

        preview.is_dirty = false;
        if let Some(error) = preview_error {
            self.push_log(error, LogLevel::Error);
        }
    }
}
