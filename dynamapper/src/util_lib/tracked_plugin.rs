use bevy::app::Plugin;
use crate::{core::system_sets::StartupSysSet, console_logger::{self, LogAbout, LogSev}};
use std::{collections::{HashMap, HashSet}, sync::{Mutex, OnceLock}};


#[derive(Debug, Default)]
struct PluginRegistryNode {
    parent: Option<String>,
    children: Vec<String>,
    seen_as_plugin: bool,
}

#[derive(Debug, Default)]
struct PluginRegistry {
    nodes: HashMap<String, PluginRegistryNode>,
    roots: Vec<String>,
}

static PLUGIN_REGISTRY: OnceLock<Mutex<PluginRegistry>> = OnceLock::new();

fn plugin_registry() -> &'static Mutex<PluginRegistry> {
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
    fn record(&mut self, plugin_name: &str, registered_by: &str) {
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

        if prefix.is_empty() {
            lines.push(name.to_string());
        } else {
            let branch = if is_last { "└─ " } else { "├─ " };
            lines.push(format!("{prefix}{branch}{name}"));
        }

        if let Some(node) = self.nodes.get(name) {
            let child_prefix = if prefix.is_empty() {
                if is_last { "   ".to_string() } else { "│  ".to_string() }
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

    console_logger::one(
        LogSev::Info,
        LogAbout::Plugins,
        &format!("Build: {bare_name} (registered by: {}).", plugin.registered_by()),
    );
}

pub fn log_plugin_registry_tree() {
    let registry = plugin_registry()
        .lock()
        .expect("plugin registry poisoned");

    console_logger::one(
        LogSev::Info,
        LogAbout::Plugins,
        "Plugin registry tree:",
    );

    for line in registry.tree_lines() {
        console_logger::one(LogSev::Info, LogAbout::Plugins, &line);
    }
}

fn log_system_add_base<'a>(myname: &'static str, plugname: &str, schedule: &'static str, sys_set: &'a str) {
    let plugname_bare = plugname.rsplit("::").next().unwrap();
    let myname_bare = myname.rsplit("::").next().unwrap();
    console_logger::one(
        console_logger::LogSev::Debug,
        console_logger::LogAbout::Startup,
        &format!("Running with schedule '{schedule}' in set '{sys_set}': system '{myname_bare}' (registered by plugin: {plugname_bare})."),
    );
}

pub fn log_system_add_startup<T: TrackedPlugin>(sys_set: StartupSysSet, _myname: &'static str) {
    log_system_add_base(_myname, std::any::type_name::<T>(), "Startup", sys_set.as_ref())
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

