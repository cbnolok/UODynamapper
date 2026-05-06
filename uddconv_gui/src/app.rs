use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use eframe::egui;
use crate::models::{AppSettings, LogMessage, LogLevel, Tab};
use crate::logic::settings::{load_settings};

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
}

impl UddConvApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let settings = load_settings();

        Self {
            settings,
            logs: Arc::new(Mutex::new(vec![LogMessage {
                text: "Application started. Please configure your source directories.".to_string(),
                level: LogLevel::Info,
            }])),
            is_converting: Arc::new(Mutex::new(false)),
            current_tab: Tab::Sources,
            tool_file_1: None,
            tool_file_2: None,
            preview_path: None,
            preview_texture: None,
        }
    }

    pub fn get_output_path(&self, filename: &str) -> PathBuf {
        if !self.settings.output_uddp_dir.exists() {
            let _ = std::fs::create_dir_all(&self.settings.output_uddp_dir);
        }
        self.settings.output_uddp_dir.join(filename)
    }

    pub fn get_input_uddp_path(&self, filename: &str) -> PathBuf {
        self.settings.input_uddp_dir.join(filename)
    }
}
