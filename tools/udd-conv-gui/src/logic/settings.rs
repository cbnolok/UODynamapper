use std::path::PathBuf;
use color_eyre::eyre;
use crate::models::AppSettings;
use uocf::utils::config_toml::{parse_app_state, AppStateLoad};

const CONFIG_FILE_NAME: &str = "udd_conv_gui_config.toml";
const CONFIG_ENV_VAR: &str = "UDD_CONV_GUI_CONFIG";

pub fn config_file_path() -> PathBuf {
    if let Some(path) = std::env::var_os(CONFIG_ENV_VAR) {
        return PathBuf::from(path);
    }
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            return parent.join(CONFIG_FILE_NAME);
        }
    }
    PathBuf::from(CONFIG_FILE_NAME)
}

/// The settings, and the warnings to show: keys filled or dropped while reading the file, or why the file could
/// not be read at all.
pub fn load_settings_report() -> (AppSettings, Vec<String>) {
    let path = config_file_path();
    match load_settings_from_path(&path) {
        Ok(load) => (load.value, load.warnings),
        Err(error) => (
            AppSettings::default(),
            vec![format!("Could not load settings: {error}. Using defaults.")],
        ),
    }
}

/// Reads the file the GUI writes itself. Keys an older file lacks take their default, unknown keys are dropped,
/// each with a warning; the GUI rewrites the file complete on its next save. A missing file gives the defaults.
pub fn load_settings_from_path(path: &std::path::Path) -> eyre::Result<AppStateLoad<AppSettings>> {
    if !path.exists() {
        return Ok(AppStateLoad { value: AppSettings::default(), warnings: Vec::new() });
    }

    let contents = std::fs::read_to_string(path)?;
    parse_app_state(&path.display().to_string(), &contents, &AppSettings::default())
}

pub fn save_settings(settings: &AppSettings) -> eyre::Result<()> {
    save_settings_to_path(settings, &config_file_path())
}

pub fn save_settings_to_path(settings: &AppSettings, path: &std::path::Path) -> eyre::Result<()> {
    let contents = toml::to_string_pretty(settings)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
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
        std::env::temp_dir().join(format!("udd_conv_gui_{}_{}.toml", name, nanos))
    }

    #[test]
    fn missing_settings_file_loads_defaults() {
        let path = temp_config_path("missing");

        let settings = load_settings_from_path(&path).expect("load default settings").value;

        assert_eq!(settings.input_uddp_dir, PathBuf::from("packages"));
        assert_eq!(settings.output_uddp_dir, PathBuf::from("packages"));
    }

    /// The checked-in file is complete: every key the GUI writes is present.
    #[test]
    fn checked_in_default_config_deserializes() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CONFIG_FILE_NAME);

        let load = load_settings_from_path(&path).expect("load checked-in default settings");
        assert!(load.warnings.is_empty(), "{:?}", load.warnings);
        let settings = load.value;

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
        settings.cc_dir = Some(PathBuf::from("/uo/classic"));
        settings.ec_dir = Some(PathBuf::from("/uo/enhanced"));
        settings.dynamapper_routing_dir = Some(PathBuf::from("dynamapper/assets/cc_ec_convtables"));
        settings.input_uddp_dir = PathBuf::from("runtime-assets");
        settings.output_uddp_dir = PathBuf::from("converted");
        settings.include_verdata = true;
        settings.ec_mobile_anim_allow_missing_kdl = true;

        save_settings_to_path(&settings, &path).expect("save settings");
        let loaded = load_settings_from_path(&path).expect("load settings").value;
        let _ = std::fs::remove_file(&path);

        assert_eq!(loaded.cc_dir, Some(PathBuf::from("/uo/classic")));
        assert_eq!(loaded.ec_dir, Some(PathBuf::from("/uo/enhanced")));
        assert_eq!(
            loaded.dynamapper_routing_dir,
            Some(PathBuf::from("dynamapper/assets/cc_ec_convtables"))
        );
        assert_eq!(loaded.input_uddp_dir, PathBuf::from("runtime-assets"));
        assert_eq!(loaded.output_uddp_dir, PathBuf::from("converted"));
        assert!(loaded.include_verdata);
        assert!(loaded.ec_mobile_anim_allow_missing_kdl);
    }

    #[test]
    fn settings_save_creates_parent_directory() {
        let dir = temp_config_path("nested").with_extension("");
        let path = dir.join(CONFIG_FILE_NAME);

        save_settings_to_path(&AppSettings::default(), &path).expect("save nested settings");
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn older_settings_file_is_filled_and_unknown_keys_dropped() {
        let path = temp_config_path("older");
        std::fs::write(&path, "input_uddp_dir = \"runtime-assets\"\nretired_option = true\n").expect("write config");

        let load = load_settings_from_path(&path).expect("lenient load");
        let _ = std::fs::remove_file(&path);

        assert_eq!(load.value.input_uddp_dir, PathBuf::from("runtime-assets"));
        assert_eq!(load.value.output_uddp_dir, AppSettings::default().output_uddp_dir);
        let name = path.display().to_string();
        assert!(load.warnings.contains(&format!("{name}: missing key `output_uddp_dir` filled from the default")));
        assert!(load.warnings.contains(&format!("{name}: unknown key `retired_option` dropped")));
    }
}
