//! Editing model of the waypoint.uop view of the UOP browser: the waypoint records of `waypoint.bin` with undo and a
//! save that writes a copy of the client package. The definitions, unknown fields and trailing bytes are kept as
//! loaded. No egui here; `ui/uop_browser.rs` draws it.

use std::path::{Path, PathBuf};

use uocf::enhanced::waypoints::{WaypointRecord, WaypointsPackage};

use crate::logic::keyed_edits::{KeyedEdits, KeyedStore, KeyedValues};

/// A waypoint field, for undo coalescing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaypointField {
    X,
    Y,
    Z,
    Facet,
    Type,
    Value6,
    NameCliloc,
}

/// The whole payload is one record of the store (it is small).
impl KeyedValues<(), WaypointsPackage> for WaypointsPackage {
    fn value(&self, _key: &()) -> Option<WaypointsPackage> {
        Some(self.clone())
    }
}

impl KeyedStore<(), WaypointsPackage> for WaypointsPackage {
    fn set_value(&mut self, _key: &(), value: Option<WaypointsPackage>) {
        if let Some(value) = value {
            *self = value;
        }
    }
}

/// The edited `waypoint.bin` of one package.
#[derive(Debug)]
pub struct WaypointDocument {
    /// The client package the payload was read from, the source of the saved copies.
    pub source: PathBuf,
    pub package: WaypointsPackage,
    pub edits: KeyedEdits<(), WaypointsPackage, (usize, WaypointField)>,
    pub error: Option<String>,
}

impl WaypointDocument {
    pub fn new(source: PathBuf, package: WaypointsPackage) -> Self {
        Self { source, package, edits: KeyedEdits::default(), error: None }
    }

    /// Edits `field` of waypoint `index`.
    pub fn edit_waypoint(&mut self, index: usize, field: WaypointField, edit: impl FnOnce(&mut WaypointRecord)) {
        self.edits.edit(&mut self.package, (), Some((index, field)), |package| {
            if let Some(waypoint) = package.as_mut().and_then(|package| package.waypoints.get_mut(index)) {
                edit(waypoint);
            }
        });
    }

    /// Inserts a copy of waypoint `index` after it, or appends a zeroed waypoint when there is none; returns the
    /// index of the new waypoint.
    pub fn add_waypoint(&mut self, index: Option<usize>) -> usize {
        self.edits.edit(&mut self.package, (), None, |package| {
            let waypoints = &mut package.as_mut().expect("the payload is always present").waypoints;
            match index.and_then(|index| waypoints.get(index).copied().map(|waypoint| (index, waypoint))) {
                Some((index, waypoint)) => {
                    waypoints.insert(index + 1, waypoint);
                    index + 1
                }
                None => {
                    waypoints.push(new_waypoint());
                    waypoints.len() - 1
                }
            }
        })
    }

    /// Removes waypoint `index`.
    pub fn remove_waypoint(&mut self, index: usize) {
        self.edits.edit(&mut self.package, (), None, |package| {
            let waypoints = &mut package.as_mut().expect("the payload is always present").waypoints;
            if index < waypoints.len() {
                waypoints.remove(index);
            }
        });
    }

    pub fn undo(&mut self) -> bool {
        self.edits.undo(&mut self.package).is_some()
    }

    pub fn redo(&mut self) -> bool {
        self.edits.redo(&mut self.package).is_some()
    }

    /// Writes a copy of the source package to `output` with this payload.
    pub fn save(&mut self, output: &Path) -> Result<(), String> {
        self.package.write_waypoints_package(&self.source, output).map_err(|error| format!("{error:#}"))?;
        self.edits.mark_saved();
        Ok(())
    }
}

/// A waypoint at the origin of facet 0, with no type, `value_6` 1 (as on most client waypoints) and no name.
pub fn new_waypoint() -> WaypointRecord {
    const DEFAULT_VALUE_6: u16 = 1;
    WaypointRecord { x: 0, y: 0, z: 0, facet: 0, waypoint_type: 0, value_6: DEFAULT_VALUE_6, name_cliloc: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> WaypointDocument {
        let package = WaypointsPackage {
            version: uocf::enhanced::waypoints::WAYPOINTS_SUPPORTED_VERSION,
            icon_definitions: Vec::new(),
            effect_definitions: Vec::new(),
            type_definitions: Vec::new(),
            waypoints: vec![new_waypoint()],
            trailing_bytes: Vec::new(),
        };
        WaypointDocument::new(PathBuf::new(), package)
    }

    #[test]
    fn waypoints_are_edited_added_removed_and_undone() {
        let mut document = document();
        document.edit_waypoint(0, WaypointField::X, |waypoint| waypoint.x = 10);
        document.edit_waypoint(0, WaypointField::X, |waypoint| waypoint.x = 100);
        assert_eq!(document.add_waypoint(Some(0)), 1);
        assert_eq!(document.package.waypoints[1].x, 100);
        assert_eq!(document.add_waypoint(None), 2);
        document.remove_waypoint(0);
        assert_eq!(document.package.waypoints.len(), 2);
        assert!(document.edits.is_dirty());
        while document.undo() {}
        assert_eq!(document.package.waypoints, vec![new_waypoint()], "the typed x is one undo step");
        assert!(!document.edits.is_dirty());
        assert!(document.redo());
        assert_eq!(document.package.waypoints[0].x, 100);
    }

    #[test]
    fn saving_needs_a_source_package() {
        let mut document = document();
        assert!(document.save(Path::new("waypoint.uop")).is_err());
    }
}
