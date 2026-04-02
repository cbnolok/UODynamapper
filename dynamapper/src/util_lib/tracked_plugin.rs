use crate::{
    console_logger::{self, LogAbout, LogSev},
    core::system_sets::StartupSysSet,
};
use bevy::app::Plugin;
use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, OnceLock},
};

#[derive(Debug, Default)]
struct PluginRegistryNode {
    parent: Option<String>,
    children: Vec<String>,
    seen_as_plugin: bool,
}

#[derive(Debug, Default)]
pub struct PluginRegistry {
    nodes: HashMap<String, PluginRegistryNode>,
    roots: Vec<String>,
}

static PLUGIN_REGISTRY: OnceLock<Mutex<PluginRegistry>> = OnceLock::new();
static PLUGIN_LOG_TOGGLES: OnceLock<std::sync::Mutex<PluginLogToggles>> = OnceLock::new();

#[derive(Debug)]
struct PluginLogToggles {
    emit_flat: bool,
    emit_tree: bool,
}

pub fn set_plugin_log_toggles(flat: bool, tree: bool) {
    let m = PLUGIN_LOG_TOGGLES.get_or_init(|| {
        std::sync::Mutex::new(PluginLogToggles {
            emit_flat: flat,
            emit_tree: tree,
        })
    });
    if let Ok(mut guard) = m.lock() {
        guard.emit_flat = flat;
        guard.emit_tree = tree;
    }
}

fn emit_flat_enabled() -> bool {
    PLUGIN_LOG_TOGGLES
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.emit_flat))
        .unwrap_or(true)
}

fn emit_tree_enabled() -> bool {
    PLUGIN_LOG_TOGGLES
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.emit_tree))
        .unwrap_or(true)
}

pub fn plugin_registry() -> &'static Mutex<PluginRegistry> {
    PLUGIN_REGISTRY.get_or_init(|| Mutex::new(PluginRegistry::default()))
}

fn bare_name(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

fn push_unique(list: &mut Vec<String>, value: &str) {
    if !list.iter().any(|existing| existing == value) {
        list.push(value.to_string());
    }
}

impl PluginRegistry {
    pub fn record(&mut self, plugin_name: &str, registered_by: &str) {
        let plugin_name = bare_name(plugin_name).to_string();
        let registered_by = bare_name(registered_by).to_string();

        let parent_is_root = self
            .nodes
            .get(&registered_by)
            .is_none_or(|node| node.parent.is_none());

        let node = self.nodes.entry(plugin_name.clone()).or_default();
        node.seen_as_plugin = true;
        node.parent = Some(registered_by.clone());

        let parent = self.nodes.entry(registered_by.clone()).or_default();
        push_unique(&mut parent.children, &plugin_name);

        self.roots.retain(|root| root != &plugin_name);
        if parent_is_root {
            push_unique(&mut self.roots, &registered_by);
        } else {
            self.roots.retain(|root| root != &registered_by);
        }
    }

    fn tree_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let mut visited = HashSet::new();

        for root in &self.roots {
            self.push_root_tree_lines(root, &mut visited, &mut lines);
        }

        let mut orphan_names: Vec<_> = self
            .nodes
            .keys()
            .filter(|name| !visited.contains(*name))
            .cloned()
            .collect();
        orphan_names.sort();

        for orphan in orphan_names {
            self.push_root_tree_lines(&orphan, &mut visited, &mut lines);
        }

        lines
    }

    fn push_root_tree_lines(
        &self,
        name: &str,
        visited: &mut HashSet<String>,
        lines: &mut Vec<String>,
    ) {
        if !visited.insert(name.to_string()) {
            return;
        }

        lines.push(name.to_string());

        if let Some(node) = self.nodes.get(name) {
            for (index, child) in node.children.iter().enumerate() {
                let child_is_last = index + 1 == node.children.len();
                self.push_tree_lines(child, "", child_is_last, visited, lines);
            }
        }
    }

    fn push_tree_lines(
        &self,
        name: &str,
        prefix: &str,
        is_last: bool,
        visited: &mut HashSet<String>,
        lines: &mut Vec<String>,
    ) {
        if !visited.insert(name.to_string()) {
            return;
        }

        let branch = if is_last { "└─ " } else { "├─ " };
        if prefix.is_empty() {
            lines.push(format!("{branch}{name}"));
        } else {
            lines.push(format!("{prefix}{branch}{name}"));
        }

        if let Some(node) = self.nodes.get(name) {
            let child_prefix = if prefix.is_empty() {
                if is_last {
                    "   ".to_string()
                } else {
                    "│  ".to_string()
                }
            } else if is_last {
                format!("{prefix}   ")
            } else {
                format!("{prefix}│  ")
            };

            for (index, child) in node.children.iter().enumerate() {
                let child_is_last = index + 1 == node.children.len();
                self.push_tree_lines(child, &child_prefix, child_is_last, visited, lines);
            }
        }
    }
}

pub fn log_plugin_build<T: TrackedPlugin>(plugin: &T) {
    let full_name = std::any::type_name::<T>();
    let bare_name = full_name.rsplit("::").next().unwrap();

    plugin_registry()
        .lock()
        .expect("plugin registry poisoned")
        .record(bare_name, plugin.registered_by());

    if emit_flat_enabled() {
        console_logger::one(
            LogSev::Debug,
            LogAbout::Plugins,
            &format!(
                "Build: {bare_name} (registered by: {}).",
                plugin.registered_by()
            ),
        );
    }
}

pub fn sys_log_plugin_registry_tree() {
    log_system_add_base(
        "sys_log_plugin_registry_tree",
        "crate::core::RenderPlugin",
        "Startup",
        "Done",
    );
    if !emit_tree_enabled() {
        return;
    }

    let registry = plugin_registry().lock().expect("plugin registry poisoned");

    console_logger::one(LogSev::Debug, LogAbout::Plugins, "Plugin registry tree:");

    for line in registry.tree_lines() {
        console_logger::one(LogSev::Debug, LogAbout::Plugins, &line);
    }
}

#[track_caller]
fn log_system_add_base<'a>(
    myname: &'static str,
    plugname: &str,
    schedule: &str,
    sys_set: &'a str,
) {
    let plugname_bare = plugname.rsplit("::").next().unwrap();
    let myname_bare = myname.rsplit("::").next().unwrap();
    console_logger::one(
        console_logger::LogSev::Debug,
        console_logger::LogAbout::Startup,
        &format!("Running with schedule '{schedule}' in set '{sys_set}': system '{myname_bare}' (registered by plugin: {plugname_bare})."),
    );
}

#[track_caller]
pub fn log_system_add_startup<T: TrackedPlugin>(sys_set: StartupSysSet, _myname: &'static str) {
    log_system_add_base(
        _myname,
        std::any::type_name::<T>(),
        "Startup",
        sys_set.as_ref(),
    )
}

#[track_caller]
pub fn log_system_add_one_shot<T: TrackedPlugin>(
    schedule: &str,
    sys_set: &str,
    _myname: &'static str,
) {
    log_system_add_base(_myname, std::any::type_name::<T>(), schedule, sys_set)
}

pub fn log_system_add_update<T: TrackedPlugin>(_myname: &'static str) {
    // do nothing for now, it can be too cluttering.
    //log_system_add_base(_myname, std::any::type_name::<T>(), "Update")
}

pub trait TrackedPlugin: Plugin {
    fn registered_by(&self) -> &str;
}

#[macro_export]
macro_rules! impl_tracked_plugin {
    ($plugin:ty) => {
        impl TrackedPlugin for $plugin {
            fn registered_by(&self) -> &str {
                self.registered_by
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_tree_unicode() {
        // reset singleton registry state for test
        let mut reg = plugin_registry().lock().expect("poisoned");
        reg.nodes.clear();
        reg.roots.clear();

        // build a small plugin graph
        reg.record("ExternalDataPlugin", "Core");
        reg.record("SettingsPlugin", "ExternalDataPlugin");
        reg.record("ShaderPresetsPlugin", "ExternalDataPlugin");
        reg.record("ControlsPlugin", "Core");
        reg.record("PlayerMovementPlugin", "ControlsPlugin");
        reg.record("RenderPlugin", "Core");
        reg.record("ScenePlugin", "RenderPlugin");
        reg.record("WorldPlugin", "ScenePlugin");

        let lines = reg.tree_lines();

        let expected: Vec<String> = vec![
            "Core".to_string(),
            "├─ ExternalDataPlugin".to_string(),
            "│  ├─ SettingsPlugin".to_string(),
            "│  └─ ShaderPresetsPlugin".to_string(),
            "├─ ControlsPlugin".to_string(),
            "│  └─ PlayerMovementPlugin".to_string(),
            "└─ RenderPlugin".to_string(),
            "   └─ ScenePlugin".to_string(),
            "      └─ WorldPlugin".to_string(),
        ];

        assert_eq!(lines, expected);
    }
}
