//! Reading the workspace's TOML configuration files.
//!
//! - Hand-written files (settings bundled with an app, profiles, tool configs) are read with [`parse_strict`]: their
//!   structs use `#[serde(deny_unknown_fields)]`, so an unknown key is an error, and a missing required key is an
//!   error. The message names the file, the line and column, the key and the allowed keys.
//! - Files an app writes itself and nobody edits by hand (session state, a GUI's remembered settings) are read with
//!   [`parse_app_state`]: a key missing from the file is filled from the defaults, an unknown key is dropped and a key
//!   with a value the struct rejects takes its default, each with a warning, so an older or newer build's file still
//!   loads with every other key as saved. The app rewrites the file complete on its next save.

use color_eyre::eyre;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Parses a hand-written TOML file. Errors start with `file_name`.
pub fn parse_strict<T: DeserializeOwned>(file_name: &str, text: &str) -> eyre::Result<T> {
    toml::from_str(text).map_err(|error| eyre::eyre!("{file_name}: {}", error.to_string().trim_end()))
}

/// An app-written file as read, and what was repaired.
#[derive(Debug)]
pub struct AppStateLoad<T> {
    pub value: T,
    /// One line per filled or dropped key. Non-empty means the file should be rewritten.
    pub warnings: Vec<String>,
}

/// Parses a file the app writes itself. Keys missing from the file take their value from `defaults`, unknown keys
/// are dropped, and a key whose value the struct rejects (an unknown enum variant, a value of the wrong type) takes
/// its default alone; each is reported in [`AppStateLoad::warnings`].
///
/// Errors (invalid TOML, values that are valid one by one but not together) start with `file_name`; the caller falls
/// back to its defaults.
pub fn parse_app_state<T>(file_name: &str, text: &str, defaults: &T) -> eyre::Result<AppStateLoad<T>>
where
    T: Serialize + DeserializeOwned,
{
    let file: toml::Table = parse_strict(file_name, text)?;
    let defaults = toml::Table::try_from(defaults)
        .map_err(|error| eyre::eyre!("{file_name}: cannot serialize the defaults: {error}"))?;

    let mut warnings = Vec::new();
    let mut extras = Vec::new();
    let merged = merge(file, &defaults, &[], &mut warnings, &mut extras);

    let mut accepted = without(&merged, &extras);
    // A value the struct rejects (an enum variant only another build knows, a value of the wrong type) resets its own
    // key to the default; every other key keeps the file's value. Each changed key is tried alone among the defaults.
    if T::deserialize(accepted.clone()).is_err() {
        let mut changed = Vec::new();
        changed_values(&accepted, &defaults, &[], &mut changed);
        for (path, value, default) in changed {
            let mut candidate = defaults.clone();
            insert(&mut candidate, &path, value.clone());
            if let Err(error) = T::deserialize(candidate) {
                let reason = error.to_string().trim_end().replace('\n', " ");
                warnings.push(format!("invalid value {value} of key `{}` reset to the default ({reason})", path.join(".")));
                insert(&mut accepted, &path, default);
            }
        }
    }

    // Keys the defaults do not serialize: unknown keys, or optional keys whose default is unset. Keep each one the
    // struct accepts.
    for (path, value) in &extras {
        let mut candidate = accepted.clone();
        insert(&mut candidate, path, value.clone());
        if deserialize::<T>(file_name, candidate.clone()).is_ok() {
            accepted = candidate;
        } else {
            warnings.push(format!("unknown key `{}` dropped", path.join(".")));
        }
    }

    let value = deserialize(file_name, accepted)?;
    let warnings = warnings.into_iter().map(|warning| format!("{file_name}: {warning}")).collect();
    Ok(AppStateLoad { value, warnings })
}

fn deserialize<T: DeserializeOwned>(file_name: &str, table: toml::Table) -> eyre::Result<T> {
    T::deserialize(table).map_err(|error| eyre::eyre!("{file_name}: {}", error.to_string().trim_end()))
}

type KeyPath = Vec<String>;

/// The file's table with every key it lacks taken from `defaults`; tables merge recursively, other values (arrays
/// included) are taken whole. Keys only the file has are moved to `extras`.
fn merge(
    mut file: toml::Table,
    defaults: &toml::Table,
    path: &[String],
    warnings: &mut Vec<String>,
    extras: &mut Vec<(KeyPath, toml::Value)>,
) -> toml::Table {
    let mut merged = toml::Table::new();
    for (key, default) in defaults {
        let key_path = [path, &[key.clone()]].concat();
        let value = match (file.remove(key), default) {
            (Some(toml::Value::Table(table)), toml::Value::Table(default)) => {
                toml::Value::Table(merge(table, default, &key_path, warnings, extras))
            }
            (Some(value), _) => value,
            (None, default) => {
                warnings.push(format!("missing key `{}` filled from the default", key_path.join(".")));
                default.clone()
            }
        };
        merged.insert(key.clone(), value);
    }
    for (key, value) in file {
        extras.push(([path, &[key]].concat(), value));
    }
    merged
}

/// Each value of `table` that differs from its default, with its key path and the default; tables that have a default
/// table are walked, other values (arrays included) are taken whole, as `merge` takes them. Keys without a default
/// are skipped.
fn changed_values(
    table: &toml::Table,
    defaults: &toml::Table,
    path: &[String],
    changed: &mut Vec<(KeyPath, toml::Value, toml::Value)>,
) {
    for (key, value) in table {
        let Some(default) = defaults.get(key) else {
            continue;
        };
        let key_path = [path, &[key.clone()]].concat();
        match (value, default) {
            (toml::Value::Table(table), toml::Value::Table(defaults)) => {
                changed_values(table, defaults, &key_path, changed);
            }
            _ if value != default => changed.push((key_path, value.clone(), default.clone())),
            _ => {}
        }
    }
}

fn without(table: &toml::Table, extras: &[(KeyPath, toml::Value)]) -> toml::Table {
    let mut table = table.clone();
    for (path, _) in extras {
        if let Some((key, parents)) = path.split_last() {
            if let Some(parent) = table_at(&mut table, parents) {
                parent.remove(key);
            }
        }
    }
    table
}

fn insert(table: &mut toml::Table, path: &[String], value: toml::Value) {
    if let Some((key, parents)) = path.split_last() {
        if let Some(parent) = table_at(table, parents) {
            parent.insert(key.clone(), value);
        }
    }
}

fn table_at<'a>(table: &'a mut toml::Table, path: &[String]) -> Option<&'a mut toml::Table> {
    path.iter().try_fold(table, |table, key| table.get_mut(key)?.as_table_mut())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct State {
        zoom: f32,
        window: Window,
        last_path: Option<String>,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Window {
        width: u32,
        height: u32,
    }

    const DEFAULT_ZOOM: f32 = 1.0;
    const DEFAULT_WIDTH: u32 = 800;
    const DEFAULT_HEIGHT: u32 = 600;

    fn defaults() -> State {
        State { zoom: DEFAULT_ZOOM, window: Window { width: DEFAULT_WIDTH, height: DEFAULT_HEIGHT }, last_path: None }
    }

    #[test]
    fn strict_errors_name_the_file_the_position_and_the_allowed_keys() {
        let error = parse_strict::<Window>("window.toml", "width = 1\nheight = 2\ndepth = 3\n")
            .expect_err("unknown key")
            .to_string();
        assert!(error.starts_with("window.toml: TOML parse error at line 3, column 1"), "{error}");
        assert!(error.contains("unknown field `depth`, expected `width` or `height`"), "{error}");

        let error = parse_strict::<Window>("window.toml", "width = 1\n").expect_err("missing key").to_string();
        assert!(error.contains("missing field `height`"), "{error}");
    }

    #[test]
    fn app_state_fills_missing_keys_from_the_defaults() {
        let load = parse_app_state("state.toml", "zoom = 2.0\n[window]\nwidth = 1024\n", &defaults()).expect("load");
        assert_eq!(
            load.value,
            State { zoom: 2.0, window: Window { width: 1024, height: DEFAULT_HEIGHT }, last_path: None }
        );
        assert_eq!(load.warnings, ["state.toml: missing key `window.height` filled from the default"]);
    }

    #[test]
    fn app_state_drops_unknown_keys_and_keeps_optional_ones() {
        let text = "zoom = 2.0\nretired = true\nlast_path = \"a\"\n[window]\nwidth = 1\nheight = 2\nmaximized = true\n";
        let load = parse_app_state("state.toml", text, &defaults()).expect("load");
        assert_eq!(
            load.value,
            State { zoom: 2.0, window: Window { width: 1, height: 2 }, last_path: Some("a".into()) }
        );
        assert_eq!(
            load.warnings,
            ["state.toml: unknown key `window.maximized` dropped", "state.toml: unknown key `retired` dropped"]
        );
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Picks {
        source: Source,
        vsync: bool,
        window: Window,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum Source {
        Classic,
        Enhanced,
    }

    #[test]
    fn app_state_resets_only_the_keys_with_invalid_values() {
        let defaults = Picks {
            source: Source::Classic,
            vsync: false,
            window: Window { width: DEFAULT_WIDTH, height: DEFAULT_HEIGHT },
        };
        // An enum variant only another build knows, and a value of the wrong type, next to valid saved values.
        let text = "source = \"kingdom\"\nvsync = true\n[window]\nwidth = \"wide\"\nheight = 2\n";
        let load = parse_app_state("picks.toml", text, &defaults).expect("load");
        assert_eq!(
            load.value,
            Picks { source: Source::Classic, vsync: true, window: Window { width: DEFAULT_WIDTH, height: 2 } }
        );
        assert_eq!(load.warnings.len(), 2, "{:?}", load.warnings);
        assert!(load.warnings[0].starts_with("picks.toml: invalid value \"kingdom\" of key `source` reset"), "{:?}", load.warnings);
        assert!(load.warnings[0].contains("unknown variant `kingdom`"), "{:?}", load.warnings);
        assert!(load.warnings[1].starts_with("picks.toml: invalid value \"wide\" of key `window.width` reset"), "{:?}", load.warnings);

        let valid = "source = \"enhanced\"\nvsync = true\n[window]\nwidth = 1\nheight = 2\n";
        let load = parse_app_state("picks.toml", valid, &defaults).expect("load");
        assert_eq!(load.value, Picks { source: Source::Enhanced, vsync: true, window: Window { width: 1, height: 2 } });
        assert!(load.warnings.is_empty(), "{:?}", load.warnings);
    }

    #[test]
    fn app_state_resets_a_value_of_the_wrong_type_and_rejects_invalid_toml() {
        let load = parse_app_state("state.toml", "zoom = \"big\"\n[window]\nwidth = 1\nheight = 2\n", &defaults())
            .expect("load");
        assert_eq!(load.value, State { zoom: DEFAULT_ZOOM, window: Window { width: 1, height: 2 }, last_path: None });
        assert_eq!(load.warnings.len(), 1, "{:?}", load.warnings);
        let error = parse_app_state("state.toml", "zoom = \n", &defaults()).expect_err("invalid TOML").to_string();
        assert!(error.starts_with("state.toml: "), "{error}");
    }
}
