//! Editing model of the LegacyTerrain.uop view of the UOP browser: every definition of the package by land tile id,
//! with undo and a save that writes a copy of the client package with only the changed entries (each document
//! keeps its formatting, so unedited ones keep their bytes). No egui here; `ui/legacy_terrain.rs` draws it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use uocf::enhanced::legacy_terrain::{
    read_legacy_terrain_documents, write_legacy_terrain_package, LegacyTerrainChanges, LegacyTerrainDocument,
    LegacyTerrainFlag, LegacyTerrainType,
};
use uocf::uop_container::package::UopPackage;

use crate::logic::keyed_edits::{KeyedEdits, KeyedStore, KeyedValues};

/// A definition field, for undo coalescing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyTerrainField {
    Name,
    TexMap,
    Flag(LegacyTerrainFlag),
}

/// The definitions by land tile id.
pub type LegacyTerrainDocuments = BTreeMap<u32, LegacyTerrainDocument>;

impl KeyedValues<u32, LegacyTerrainDocument> for LegacyTerrainDocuments {
    fn value(&self, key: &u32) -> Option<LegacyTerrainDocument> {
        self.get(key).cloned()
    }
}

impl KeyedStore<u32, LegacyTerrainDocument> for LegacyTerrainDocuments {
    fn set_value(&mut self, key: &u32, value: Option<LegacyTerrainDocument>) {
        match value {
            Some(value) => {
                self.insert(*key, value);
            }
            None => {
                self.remove(key);
            }
        }
    }
}

/// The edited definitions of one package.
#[derive(Debug)]
pub struct LegacyTerrainEditor {
    /// The client package the definitions were read from, the source of the saved copies.
    pub source: PathBuf,
    pub documents: LegacyTerrainDocuments,
    pub edits: KeyedEdits<u32, LegacyTerrainDocument, LegacyTerrainField>,
    /// Shows only definitions whose id or name contains it.
    pub filter: String,
    /// Land tile id typed for a new or duplicated definition.
    pub new_id: u32,
    pub error: Option<String>,
}

impl LegacyTerrainEditor {
    /// Reads every definition of `package`, loaded from `source`.
    pub fn open(source: PathBuf, package: &UopPackage) -> Result<Self, String> {
        let documents = read_legacy_terrain_documents(package).map_err(|error| format!("{error:#}"))?;
        Ok(Self { source, documents, edits: KeyedEdits::default(), filter: String::new(), new_id: 0, error: None })
    }

    /// Edits `field` of the definition of land tile `id`.
    pub fn edit(&mut self, id: u32, field: LegacyTerrainField, edit: impl FnOnce(&mut LegacyTerrainType)) {
        self.edits.edit(&mut self.documents, id, Some(field), |document| {
            if let Some(document) = document.as_mut() {
                edit(&mut document.terrain);
            }
        });
    }

    /// Adds the definition of land tile `new_id`: a copy of `template`'s, formatted like it, or an empty one
    /// formatted like most client files.
    pub fn add(&mut self, new_id: u32, template: Option<u32>) -> Result<(), String> {
        if self.documents.contains_key(&new_id) {
            return Err(format!("land tile {new_id} already has a definition"));
        }
        let document = match template {
            Some(id) => {
                let template = self.documents.get(&id).ok_or("the definition to duplicate is missing")?;
                template.with_terrain(LegacyTerrainType { id: new_id, ..template.terrain.clone() })
            }
            None => LegacyTerrainDocument::new(LegacyTerrainType { id: new_id, ..LegacyTerrainType::default() }),
        };
        self.edits.edit(&mut self.documents, new_id, None, |slot| *slot = Some(document));
        Ok(())
    }

    /// Removes the definition of land tile `id`.
    pub fn remove(&mut self, id: u32) {
        self.edits.edit(&mut self.documents, id, None, |document| *document = None);
    }

    pub fn undo(&mut self) -> bool {
        self.edits.undo(&mut self.documents).is_some()
    }

    pub fn redo(&mut self) -> bool {
        self.edits.redo(&mut self.documents).is_some()
    }

    /// The ids of the definitions the filter shows, in id order.
    pub fn visible_ids(&self) -> Vec<u32> {
        let filter = self.filter.trim().to_lowercase();
        self.documents
            .values()
            .filter(|document| {
                filter.is_empty()
                    || document.terrain.id.to_string().contains(&filter)
                    || document.terrain.name.to_lowercase().contains(&filter)
            })
            .map(|document| document.terrain.id)
            .collect()
    }

    /// Writes a copy of the source package to `output` with the definitions changed since it was opened.
    pub fn save(&mut self, output: &Path) -> Result<LegacyTerrainChanges, String> {
        let (mut documents, mut removed) = (Vec::new(), Vec::new());
        for (id, document) in self.edits.changes_since_opened(&self.documents) {
            match document {
                Some(document) => documents.push(document),
                None => removed.push(id),
            }
        }
        let changes = write_legacy_terrain_package(&self.source, output, &documents, &removed)
            .map_err(|error| format!("{error:#}"))?;
        self.edits.mark_saved();
        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use uocf::enhanced::legacy_terrain::legacy_terrain_entry_path;
    use uocf::uop_container::file::CompressionFlag;

    const SAND: u32 = 969;
    const ROCK: u32 = 1417;
    const NEW_TILE: u32 = 2000;

    fn editor() -> LegacyTerrainEditor {
        let mut package = UopPackage::new_default();
        for (id, name) in [(SAND, "sand"), (ROCK, "rock")] {
            let terrain = LegacyTerrainType { id, name: name.to_owned(), tex_map: id, flags: Vec::new() };
            let path = legacy_terrain_entry_path(id);
            package
                .add_file_from_memory(&LegacyTerrainDocument::new(terrain).to_bytes(), &path, CompressionFlag::Zlib)
                .unwrap();
        }
        LegacyTerrainEditor::open(PathBuf::new(), &package).unwrap()
    }

    #[test]
    fn definitions_are_edited_added_removed_and_undone() {
        let mut editor = editor();
        editor.edit(SAND, LegacyTerrainField::Name, |terrain| terrain.name.push('s'));
        editor.edit(SAND, LegacyTerrainField::Name, |terrain| terrain.name.push('!'));
        editor.edit(SAND, LegacyTerrainField::Flag(LegacyTerrainFlag::Wet), |terrain| {
            terrain.set_flag(LegacyTerrainFlag::Wet, true)
        });
        assert!(editor.add(SAND, None).is_err());
        editor.add(NEW_TILE, Some(SAND)).unwrap();
        assert_eq!(editor.documents[&NEW_TILE].terrain.name, "sands!");
        assert!(editor.documents[&NEW_TILE].terrain.has_flag(LegacyTerrainFlag::Wet));
        editor.remove(ROCK);
        editor.filter = "SAND".to_owned();
        assert_eq!(editor.visible_ids(), [SAND, NEW_TILE]);

        let changes = editor.edits.changes_since_opened(&editor.documents);
        assert_eq!(changes.keys().copied().collect::<Vec<_>>(), [SAND, ROCK, NEW_TILE]);
        assert!(changes[&ROCK].is_none());
        while editor.undo() {}
        assert_eq!(editor.documents[&SAND].terrain.name, "sand", "the typed name is one undo step");
        assert!(editor.documents.contains_key(&ROCK) && !editor.documents.contains_key(&NEW_TILE));
        assert!(!editor.edits.is_dirty());
        assert!(editor.redo());
        assert_eq!(editor.documents[&SAND].terrain.name, "sands!");
    }

    #[test]
    fn saving_needs_a_source_package() {
        let mut editor = editor();
        assert!(editor.save(Path::new("LegacyTerrain.uop")).is_err());
    }
}
