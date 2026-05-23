use std::path::PathBuf;
use color_eyre::eyre;
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

pub fn load_settings_report() -> (AppSettings, Option<String>) {
    let path = config_file_path();
    match load_settings_from_path(&path) {
        Ok(settings) => (settings, None),
        Err(error) => (
            AppSettings::default(),
            Some(format!(
                "Could not load settings from {}: {}. Using defaults.",
                path.display(),
                error
            )),
        ),
    }
}

pub fn load_settings_from_path(path: &std::path::Path) -> eyre::Result<AppSettings> {
    if !path.exists() {
        return Ok(AppSettings::default());
    }

    let contents = std::fs::read_to_string(path)?;
    let settings = toml::from_str(&contents)?;
    Ok(settings)
}

pub fn save_settings(settings: &AppSettings) -> eyre::Result<()> {
    save_settings_to_path(settings, &config_file_path())
}

pub fn save_settings_to_path(settings: &AppSettings, path: &std::path::Path) -> eyre::Result<()> {
    let contents = toml::to_string_pretty(settings)?;
    std::fs::write(path, contents)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time is after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("uddconv_gui_{}_{}.toml", name, nanos))
    }

    #[test]
    fn missing_settings_file_loads_defaults() {
        let path = temp_config_path("missing");

        let settings = load_settings_from_path(&path).expect("load default settings");

        assert_eq!(settings.input_uddp_dir, PathBuf::from("packages"));
        assert_eq!(settings.output_uddp_dir, PathBuf::from("packages"));
    }

    #[test]
    fn invalid_settings_file_reports_error() {
        let path = temp_config_path("invalid");
        std::fs::write(&path, "not valid toml =").expect("write invalid config");

        let result = load_settings_from_path(&path);
        let _ = std::fs::remove_file(&path);

        assert!(result.is_err());
    }

    #[test]
    fn settings_save_and_load_roundtrip() {
        let path = temp_config_path("roundtrip");
        let mut settings = AppSettings::default();
        settings.output_uddp_dir = PathBuf::from("converted");
        settings.include_verdata = true;

        save_settings_to_path(&settings, &path).expect("save settings");
        let loaded = load_settings_from_path(&path).expect("load settings");
        let _ = std::fs::remove_file(&path);

        assert_eq!(loaded.output_uddp_dir, PathBuf::from("converted"));
        assert!(loaded.include_verdata);
    }
}
