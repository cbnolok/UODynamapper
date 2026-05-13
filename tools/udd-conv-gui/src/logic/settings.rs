use std::path::PathBuf;
use crate::models::AppSettings;

const CONFIG_FILE_NAME: &str = "uddconv_gui_config.toml";

pub fn config_file_path() -> PathBuf {
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        return PathBuf::from(manifest_dir).join(CONFIG_FILE_NAME);
    }
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            return parent.join(CONFIG_FILE_NAME);
        }
    }
    PathBuf::from(CONFIG_FILE_NAME)
}

pub fn load_settings() -> AppSettings {
    let path = config_file_path();
    if let Ok(contents) = std::fs::read_to_string(&path) {
        toml::from_str(&contents).unwrap_or_default()
    } else {
        AppSettings::default()
    }
}

pub fn save_settings(settings: &AppSettings) {
    let path = config_file_path();
    if let Ok(contents) = toml::to_string_pretty(settings) {
        let _ = std::fs::write(path, contents);
    }
}
