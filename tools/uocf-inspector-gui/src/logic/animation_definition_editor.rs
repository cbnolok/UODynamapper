//! Editing model of the Animation definitions tab: every entry of an `AnimationDefinition.uop` by item id, in the
//! package's dialect (Enhanced Client or Kingdom Reborn), with undo, the client checks, KDL import and export and a
//! save that writes a copy of the package with only the changed entries. No egui here;
//! `ui/animation_definitions.rs` draws it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use uocf::enhanced::animation_definition::{
    animation_definitions_from_kdl, animation_definitions_to_kdl, AnimationDefinition, AnimationDefinitionChanges,
    AnimationDefinitionCollection, AnimationDefinitionImport, AnimationDefinitionIssue, AnimationIssueSeverity,
    DefinitionFrame, FrameSet,
};
use uocf::enhanced::client_dialect::ClientDialect;
use uocf::enhanced::effect_definition::{effect_definitions_from_kdl, effect_definitions_to_kdl, EffectValidationContext};
use uocf::enhanced::string_dictionary::UoStringDictionary;

use crate::logic::keyed_edits::{KeyedEdits, KeyedStore, KeyedValues};

/// Frame interval of a set added to a definition without sets (the most common client interval).
pub const NEW_SET_FRAME_INTERVAL: f32 = 0.25;

/// A definition field, for undo coalescing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationDefinitionField {
    StoredId,
    StartSet,
    SecondSet,
    Value,
    Interval(u32),
    Next(u32),
    FrameTile(u32, usize),
    FrameEffects(u32, usize),
    /// Sets or frames added, removed or moved.
    Structure,
}

/// The definitions by item id.
pub type AnimationDefinitions = BTreeMap<u32, AnimationDefinition>;

impl KeyedValues<u32, AnimationDefinition> for AnimationDefinitions {
    fn value(&self, key: &u32) -> Option<AnimationDefinition> {
        self.get(key).cloned()
    }
}

impl KeyedStore<u32, AnimationDefinition> for AnimationDefinitions {
    fn set_value(&mut self, key: &u32, value: Option<AnimationDefinition>) {
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
pub struct AnimationDefinitionEditor {
    /// The package the definitions were read from, the source of the saved copies.
    pub source: PathBuf,
    /// The effect grammar of the package, kept on save.
    pub dialect: ClientDialect,
    pub definitions: AnimationDefinitions,
    pub edits: KeyedEdits<u32, AnimationDefinition, AnimationDefinitionField>,
}

impl AnimationDefinitionEditor {
    /// Reads every entry of the package `source` in `dialect`.
    pub fn open(source: PathBuf, dialect: ClientDialect) -> Result<Self, String> {
        let collection = AnimationDefinitionCollection::load_dialect(&source, dialect).map_err(|error| format!("{error:#}"))?;
        Ok(Self::from_collection(source, collection))
    }

    pub fn from_collection(source: PathBuf, collection: AnimationDefinitionCollection) -> Self {
        Self { source, dialect: collection.dialect, definitions: collection.entries, edits: KeyedEdits::default() }
    }

    /// Edits `field` of the definition of item `id`.
    pub fn edit(&mut self, id: u32, field: AnimationDefinitionField, edit: impl FnOnce(&mut AnimationDefinition)) {
        self.edits.edit(&mut self.definitions, id, Some(field), |definition| {
            if let Some(definition) = definition.as_mut() {
                edit(definition);
            }
        });
    }

    /// One undo step that changes the sets or frames of item `id`.
    fn edit_structure(&mut self, id: u32, edit: impl FnOnce(&mut AnimationDefinition)) {
        self.edits.break_coalescing();
        self.edit(id, AnimationDefinitionField::Structure, edit);
        self.edits.break_coalescing();
    }

    /// Adds a set after the highest key, a copy of the last set without a successor (one frame of the item's
    /// stored id when there is no set); returns its key.
    pub fn add_set(&mut self, id: u32) -> Option<u32> {
        let definition = self.definitions.get(&id)?;
        let key = definition.sets.keys().next_back().map_or(0, |last| last.saturating_add(1));
        let set = definition.sets.values().next_back().cloned().map(|set| FrameSet { next: None, ..set });
        let set = set.unwrap_or_else(|| FrameSet {
            next: None,
            frame_interval: NEW_SET_FRAME_INTERVAL,
            frames: vec![DefinitionFrame { tile_id: definition.stored_id, effects: Vec::new() }],
        });
        self.edit_structure(id, |definition| {
            definition.sets.insert(key, set);
        });
        Some(key)
    }

    /// Removes set `key` of item `id`; links to it are kept and reported by the validation.
    pub fn remove_set(&mut self, id: u32, key: u32) {
        self.edit_structure(id, |definition| {
            definition.sets.remove(&key);
        });
    }

    /// Appends a frame to set `key`: a copy of its last frame without effects, else the item's stored id.
    pub fn add_frame(&mut self, id: u32, key: u32) {
        self.edit_structure(id, |definition| {
            let stored_id = definition.stored_id;
            if let Some(set) = definition.sets.get_mut(&key) {
                let tile_id = set.frames.last().map_or(stored_id, |frame| frame.tile_id);
                set.frames.push(DefinitionFrame { tile_id, effects: Vec::new() });
            }
        });
    }

    pub fn remove_frame(&mut self, id: u32, key: u32, index: usize) {
        self.edit_structure(id, |definition| {
            if let Some(set) = definition.sets.get_mut(&key).filter(|set| index < set.frames.len()) {
                set.frames.remove(index);
            }
        });
    }

    /// Swaps frame `index` of set `key` with the previous one (`up`) or the next one.
    pub fn move_frame(&mut self, id: u32, key: u32, index: usize, up: bool) {
        self.edit_structure(id, |definition| {
            let Some(set) = definition.sets.get_mut(&key) else {
                return;
            };
            let other = if up { index.checked_sub(1) } else { index.checked_add(1) };
            if let Some(other) = other.filter(|&other| other < set.frames.len() && index < set.frames.len()) {
                set.frames.swap(index, other);
            }
        });
    }

    /// Replaces the effects of frame `index` of set `key` with the KDL text `text` (the `EffectDefinitionCollection`
    /// grammar); a parse error, with its line and column, leaves the frame as it is.
    pub fn set_frame_effects_kdl(
        &mut self,
        id: u32,
        key: u32,
        index: usize,
        text: &str,
        strings: Option<&UoStringDictionary>,
    ) -> Result<(), String> {
        let effects = effect_definitions_from_kdl(text, strings).map_err(|error| format!("{error:#}"))?;
        self.edits.break_coalescing();
        self.edit(id, AnimationDefinitionField::FrameEffects(key, index), |definition| {
            if let Some(frame) = definition.sets.get_mut(&key).and_then(|set| set.frames.get_mut(index)) {
                frame.effects = effects;
            }
        });
        self.edits.break_coalescing();
        Ok(())
    }

    /// Adds the definition of item `new_id`: one looping set of one frame showing the item.
    pub fn add_new(&mut self, new_id: u32) -> Result<(), String> {
        let frames = vec![DefinitionFrame { tile_id: new_id, effects: Vec::new() }];
        let sets = BTreeMap::from([(0, FrameSet { next: Some(0), frame_interval: NEW_SET_FRAME_INTERVAL, frames })]);
        let definition = AnimationDefinition { stored_id: new_id, start_set: 0, second_set: 0, value: 0.0, sets };
        self.insert_new(new_id, definition)
    }

    /// Adds the definition of item `new_id` as a copy of `template`'s, stored under the new id.
    pub fn duplicate(&mut self, template: u32, new_id: u32) -> Result<(), String> {
        let template = self.definitions.get(&template).ok_or("the definition to duplicate is missing")?;
        let definition = AnimationDefinition { stored_id: new_id, ..template.clone() };
        self.insert_new(new_id, definition)
    }

    fn insert_new(&mut self, new_id: u32, definition: AnimationDefinition) -> Result<(), String> {
        if !AnimationDefinitionCollection::is_valid_id(new_id) {
            return Err(format!("item {new_id} is not an item id (below 0x10000) or a mobile-keyed id"));
        }
        if self.definitions.contains_key(&new_id) {
            return Err(format!("item {new_id} already has a definition"));
        }
        self.edits.edit(&mut self.definitions, new_id, None, |slot| *slot = Some(definition));
        Ok(())
    }

    /// Removes the definition of item `id`.
    pub fn remove(&mut self, id: u32) {
        self.edits.edit(&mut self.definitions, id, None, |definition| *definition = None);
    }

    pub fn undo(&mut self) -> bool {
        self.edits.undo(&mut self.definitions).is_some()
    }

    pub fn redo(&mut self) -> bool {
        self.edits.redo(&mut self.definitions).is_some()
    }

    /// The ids of the definitions whose decimal or hex id or stored id contains `filter`, in id order.
    pub fn visible_ids(&self, filter: &str) -> Vec<u32> {
        let filter = filter.trim().trim_start_matches("0x").to_lowercase();
        let matches = |id: u32| filter.is_empty() || id.to_string().contains(&filter) || format!("{id:x}").contains(&filter);
        self.definitions
            .iter()
            .filter(|(id, definition)| matches(**id) || matches(definition.stored_id))
            .map(|(&id, _)| id)
            .collect()
    }

    /// The client checks of every definition, with their item ids.
    pub fn issues(&self, strings: Option<&UoStringDictionary>) -> Vec<AnimationDefinitionIssue> {
        let context = EffectValidationContext { strings, particle_file_exists: None };
        let mut issues = Vec::new();
        for (&id, definition) in &self.definitions {
            for mut issue in definition.validate(&context) {
                issue.item_id = Some(id);
                issues.push(issue);
            }
        }
        issues
    }

    /// Imports KDL files or folders of them: each entry they hold replaces or adds a definition (one undo step
    /// per entry). Nothing changes when a file fails.
    pub fn import_kdl(
        &mut self,
        paths: &[PathBuf],
        strings: Option<&UoStringDictionary>,
    ) -> Result<AnimationDefinitionImport, String> {
        let mut read = AnimationDefinitionCollection::new(self.dialect);
        let mut import = read.import_kdl_paths(paths, strings).map_err(|error| format!("{error:#}"))?;
        (import.replaced, import.added) = (Vec::new(), Vec::new());
        for (id, definition) in read.entries {
            if self.definitions.contains_key(&id) {
                import.replaced.push(id);
            } else {
                import.added.push(id);
            }
            self.edits.edit(&mut self.definitions, id, None, |slot| *slot = Some(definition));
        }
        Ok(import)
    }

    /// The KDL text of item `id` (one `animation` node).
    pub fn kdl_text(&self, id: u32, strings: Option<&UoStringDictionary>) -> Option<String> {
        self.definitions.get(&id).map(|definition| animation_definitions_to_kdl([(id, definition)], strings))
    }

    /// Writes the KDL file of item `id`.
    pub fn export_kdl(&self, id: u32, path: &Path, strings: Option<&UoStringDictionary>) -> Result<(), String> {
        let text = self.kdl_text(id, strings).ok_or_else(|| format!("item {id} has no definition"))?;
        std::fs::write(path, text).map_err(|error| format!("failed to write {}: {error}", path.display()))
    }

    /// Writes one KDL file per definition into `dir`; returns how many.
    pub fn export_kdl_all(&self, dir: &Path, strings: Option<&UoStringDictionary>) -> Result<usize, String> {
        self.collection().export_kdl_files(dir, strings).map_err(|error| format!("{error:#}"))
    }

    /// Writes a copy of the source package to `output` with the definitions changed since it was opened, in the
    /// package's dialect. Refused while a definition has an error.
    pub fn save(&mut self, output: &Path, strings: Option<&UoStringDictionary>) -> Result<AnimationDefinitionChanges, String> {
        let issues = self.issues(strings);
        if let Some(issue) = issues.iter().find(|issue| issue.severity() == AnimationIssueSeverity::Error) {
            return Err(format!("not saved: {issue}"));
        }
        let mut package =
            AnimationDefinitionCollection::load_dialect(&self.source, self.dialect).map_err(|error| format!("{error:#}"))?;
        for (id, definition) in self.edits.changes_since_opened(&self.definitions) {
            match definition {
                Some(definition) => {
                    package.set(id, definition);
                }
                None => {
                    package.remove(id);
                }
            }
        }
        let changes = package.write_package(&self.source, output).map_err(|error| format!("{error:#}"))?;
        self.edits.mark_saved();
        Ok(changes)
    }

    fn collection(&self) -> AnimationDefinitionCollection {
        AnimationDefinitionCollection { entries: self.definitions.clone(), ..AnimationDefinitionCollection::new(self.dialect) }
    }
}

/// The frame effects as KDL text, for the effect editor of a frame.
pub fn frame_effects_kdl(frame: &DefinitionFrame, strings: Option<&UoStringDictionary>) -> String {
    effect_definitions_to_kdl(&frame.effects, strings)
}

/// Parses the KDL text of one entry ([`AnimationDefinitionEditor::kdl_text`]), as edited in the entry's text view.
pub fn parse_single_kdl(text: &str, strings: Option<&UoStringDictionary>) -> Result<(u32, AnimationDefinition), String> {
    let mut entries = animation_definitions_from_kdl(text, strings).map_err(|error| format!("{error:#}"))?;
    match entries.len() {
        1 => Ok(entries.remove(0)),
        count => Err(format!("expected one animation node, found {count}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use uocf::uop_container::file::CompressionFlag;
    use uocf::uop_container::package::UopPackage;

    const FIRST: u32 = 0x1E5D;
    const SECOND: u32 = 0x1E60;
    const NEW_ITEM: u32 = 0x2000;
    const SOUND_ID: u32 = 42;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("uocf_inspector_animdef_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn package_path(dir: &Path) -> PathBuf {
        let mut package = UopPackage::new_default();
        let mut editor = AnimationDefinitionEditor::from_collection(PathBuf::new(), AnimationDefinitionCollection::default());
        for id in [FIRST, SECOND] {
            editor.add_new(id).unwrap();
            let payload = editor.definitions[&id].to_bytes().unwrap();
            package.add_file_from_memory(&payload, &AnimationDefinition::entry_path(id), CompressionFlag::Zlib).unwrap();
        }
        let path = dir.join("AnimationDefinition.uop");
        package.finalize_and_save(&path).unwrap();
        path
    }

    #[test]
    fn definitions_are_edited_undone_and_saved() {
        let dir = temp_dir("edit");
        let source = package_path(&dir);
        let mut editor = AnimationDefinitionEditor::open(source.clone(), ClientDialect::Enhanced).unwrap();
        assert_eq!(editor.definitions.keys().copied().collect::<Vec<_>>(), [FIRST, SECOND]);

        editor.edit(FIRST, AnimationDefinitionField::Interval(0), |definition| definition.sets.get_mut(&0).unwrap().frame_interval = 0.3);
        editor.edit(FIRST, AnimationDefinitionField::Interval(0), |definition| definition.sets.get_mut(&0).unwrap().frame_interval = 0.5);
        let key = editor.add_set(FIRST).unwrap();
        assert_eq!(key, 1);
        editor.add_frame(FIRST, key);
        editor.move_frame(FIRST, key, 1, true);
        editor.set_frame_effects_kdl(FIRST, key, 0, &format!("sound {SOUND_ID}\n"), None).unwrap();
        assert!(editor.set_frame_effects_kdl(FIRST, key, 0, "noise 1\n", None).unwrap_err().contains("line 1, column 1"));
        let set = &editor.definitions[&FIRST].sets[&key];
        assert_eq!((set.frames.len(), set.frames[0].effects.len(), set.next), (2, 1, None));
        let issues = editor.issues(None);
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].to_string().contains("never plays"), "{}", issues[0]);
        editor.edit(FIRST, AnimationDefinitionField::SecondSet, |definition| definition.second_set = key);
        assert!(editor.issues(None).is_empty());

        editor.duplicate(FIRST, NEW_ITEM).unwrap();
        assert_eq!(editor.definitions[&NEW_ITEM].stored_id, NEW_ITEM);
        assert!(editor.add_new(SECOND).is_err());
        assert!(editor.add_new(u32::MAX).is_err());
        editor.remove(SECOND);
        assert_eq!(editor.visible_ids("2000"), [NEW_ITEM]);
        assert_eq!(editor.visible_ids("1e5d"), [FIRST]);

        editor.edit(FIRST, AnimationDefinitionField::StartSet, |definition| definition.start_set = 9);
        assert!(editor.save(&dir.join("refused.uop"), None).unwrap_err().starts_with("not saved:"));
        editor.undo();
        let output = dir.join("saved.uop");
        let changes = editor.save(&output, None).unwrap();
        assert_eq!(changes, AnimationDefinitionChanges { replaced: 1, added: 1, removed: 1 });
        assert!(!editor.edits.is_dirty());
        let saved = AnimationDefinitionEditor::open(output, ClientDialect::Enhanced).unwrap();
        assert_eq!(saved.definitions, editor.definitions);

        while editor.undo() {}
        assert_eq!(editor.definitions[&FIRST].sets[&0].frame_interval, NEW_SET_FRAME_INTERVAL, "the drag is one step");
        assert_eq!(editor.definitions.keys().copied().collect::<Vec<_>>(), [FIRST, SECOND]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn kdl_export_and_import() {
        let dir = temp_dir("kdl");
        let source = package_path(&dir);
        let mut editor = AnimationDefinitionEditor::open(source, ClientDialect::Enhanced).unwrap();
        let kdl_dir = dir.join("kdl");
        assert_eq!(editor.export_kdl_all(&kdl_dir, None).unwrap(), 2);
        let one = dir.join("one.kdl");
        editor.export_kdl(FIRST, &one, None).unwrap();
        let (id, definition) = parse_single_kdl(&std::fs::read_to_string(&one).unwrap(), None).unwrap();
        assert_eq!((id, &definition), (FIRST, &editor.definitions[&FIRST]));

        let text = std::fs::read_to_string(&one).unwrap().replace(&format!("animation {FIRST}"), &format!("animation {NEW_ITEM}"));
        std::fs::write(&one, text).unwrap();
        let import = editor.import_kdl(&[one.clone(), kdl_dir], None).unwrap();
        assert_eq!((import.replaced, import.added), (vec![FIRST, SECOND], vec![NEW_ITEM]));
        assert!(editor.edits.is_dirty());
        std::fs::write(&one, "animation 1 {\n    set 0 interval=1 next=3\n}\n").unwrap();
        assert!(editor.import_kdl(&[one], None).unwrap_err().contains("line 2, column 27"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
