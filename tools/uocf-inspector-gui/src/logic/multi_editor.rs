//! Editing model of the MultiCollection.uop view of the Multis tab: edits go straight into the loaded
//! `MultiCollection` (so the list, the preview and the other tabs show them), with undo per multi and a save that
//! writes a copy of the client package. No egui here; `ui/multis.rs` draws it.

use std::path::Path;

use uocf::enhanced::multis::{MultiCollection, MultiCollectionChanges, MultiItem, MultiItemPart, MAX_MULTI_COLLECTION_ID};

use crate::logic::keyed_edits::{KeyedEdits, KeyedStore, KeyedValues};

/// A part field, for undo coalescing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiPartField {
    ItemId,
    X,
    Y,
    Z,
    NotDrawn,
    Flag2,
    StringIds,
}

/// A field of one part of a multi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiField {
    pub part: usize,
    pub field: MultiPartField,
}

impl KeyedValues<u32, MultiItem> for MultiCollection {
    fn value(&self, key: &u32) -> Option<MultiItem> {
        self.get(*key).cloned()
    }
}

impl KeyedStore<u32, MultiItem> for MultiCollection {
    fn set_value(&mut self, key: &u32, value: Option<MultiItem>) {
        match value {
            Some(item) => {
                debug_assert_eq!(item.id, *key, "a multi is stored under its id");
                self.set(item);
            }
            None => {
                self.remove(*key);
            }
        }
    }
}

/// State of the multi editor.
#[derive(Debug, Default)]
pub struct MultiEditor {
    /// The editing controls are shown.
    pub enabled: bool,
    pub edits: KeyedEdits<u32, MultiItem, MultiField>,
    pub error: Option<String>,
    /// Id typed for a new or duplicated multi.
    pub new_multi_id: u32,
}

impl MultiEditor {
    /// Edits one field of part `part` of multi `id`.
    pub fn edit_part(
        &mut self,
        collection: &mut MultiCollection,
        id: u32,
        part: usize,
        field: MultiPartField,
        edit: impl FnOnce(&mut MultiItemPart),
    ) {
        self.edits.edit(collection, id, Some(MultiField { part, field }), |item| {
            if let Some(part) = item.as_mut().and_then(|item| item.parts.get_mut(part)) {
                edit(part);
            }
        });
    }

    /// Appends `part` to multi `id`; returns its index.
    pub fn add_part(&mut self, collection: &mut MultiCollection, id: u32, part: MultiItemPart) -> Option<usize> {
        self.edits.edit(collection, id, None, |item| {
            let item = item.as_mut()?;
            item.parts.push(part);
            Some(item.parts.len() - 1)
        })
    }

    /// Inserts a copy of part `index` of multi `id` after it; returns the index of the copy.
    pub fn duplicate_part(&mut self, collection: &mut MultiCollection, id: u32, index: usize) -> Option<usize> {
        self.edits.edit(collection, id, None, |item| {
            let item = item.as_mut()?;
            let copy = item.parts.get(index)?.clone();
            item.parts.insert(index + 1, copy);
            Some(index + 1)
        })
    }

    /// Removes part `index` of multi `id`.
    pub fn remove_part(&mut self, collection: &mut MultiCollection, id: u32, index: usize) {
        self.edits.edit(collection, id, None, |item| {
            if let Some(item) = item.as_mut().filter(|item| index < item.parts.len()) {
                item.parts.remove(index);
            }
        });
    }

    /// Adds multi `id` with the parts of `template` (empty when `None`).
    pub fn add_multi(&mut self, collection: &mut MultiCollection, id: u32, template: Option<u32>) -> Result<(), String> {
        if id >= MAX_MULTI_COLLECTION_ID {
            return Err(format!("multi ids must be below {MAX_MULTI_COLLECTION_ID}"));
        }
        if collection.get(id).is_some() {
            return Err(format!("multi {id} already exists"));
        }
        let parts = match template {
            Some(template) => collection.get(template).ok_or(format!("no multi {template}"))?.parts.clone(),
            None => Vec::new(),
        };
        self.edits.edit(collection, id, None, |item| *item = Some(MultiItem::new(id, parts)));
        Ok(())
    }

    /// Removes multi `id`.
    pub fn remove_multi(&mut self, collection: &mut MultiCollection, id: u32) {
        self.edits.edit(collection, id, None, |item| *item = None);
    }
}

/// A new part: tile 0 at the multi origin, drawn, no tags.
pub fn new_part() -> MultiItemPart {
    MultiItemPart { item_id: 0, x: 0, y: 0, z: 0, not_drawn: false, flag2: 0, string_ids: Vec::new() }
}

/// The lowest multi id not in `collection`.
pub fn first_free_multi_id(collection: &MultiCollection) -> Option<u32> {
    (0..MAX_MULTI_COLLECTION_ID).find(|id| collection.get(*id).is_none())
}

/// The string ids of a part as typed: numbers separated by commas or spaces.
pub fn parse_string_ids(text: &str) -> Result<Vec<u32>, String> {
    text.split(|character: char| character == ',' || character.is_whitespace())
        .filter(|token| !token.is_empty())
        .map(|token| token.parse().map_err(|_| format!("{token:?} is not a string id")))
        .collect()
}

pub fn format_string_ids(ids: &[u32]) -> String {
    ids.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
}

/// Writes a copy of the client `source` package to `output` with the multis of `collection`
/// (`MultiCollection::write_multi_collection`: housing.bin and unchanged multis keep their bytes).
pub fn save_multi_collection(
    collection: &MultiCollection,
    source: &Path,
    output: &Path,
) -> Result<MultiCollectionChanges, String> {
    collection.write_multi_collection(source, output).map_err(|error| format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MULTI: u32 = 3;
    const TILE: u16 = 0x0064;

    fn collection() -> MultiCollection {
        let part = MultiItemPart { item_id: TILE, ..new_part() };
        MultiCollection::from_items(vec![MultiItem::new(MULTI, vec![part.clone(), part])]).unwrap()
    }

    #[test]
    fn part_edits_coalesce_and_undo() {
        let mut collection = collection();
        let mut editor = MultiEditor::default();
        editor.edit_part(&mut collection, MULTI, 0, MultiPartField::Z, |part| part.z += 1);
        editor.edit_part(&mut collection, MULTI, 0, MultiPartField::Z, |part| part.z += 1);
        editor.edit_part(&mut collection, MULTI, 1, MultiPartField::Z, |part| part.z -= 1);
        assert_eq!(collection.get(MULTI).unwrap().parts[0].z, 2);
        editor.edits.undo(&mut collection);
        editor.edits.undo(&mut collection);
        assert_eq!(collection.get(MULTI).unwrap(), self::collection().get(MULTI).unwrap());
        assert!(!editor.edits.is_dirty());
    }

    #[test]
    fn parts_and_multis_are_added_duplicated_and_removed() {
        let mut collection = collection();
        let mut editor = MultiEditor::default();
        assert_eq!(editor.add_part(&mut collection, MULTI, new_part()), Some(2));
        assert_eq!(editor.duplicate_part(&mut collection, MULTI, 0), Some(1));
        editor.remove_part(&mut collection, MULTI, 2);
        assert_eq!(collection.get(MULTI).unwrap().parts.iter().map(|part| part.item_id).collect::<Vec<_>>(), [TILE, TILE, 0]);

        let free = first_free_multi_id(&collection).unwrap();
        editor.add_multi(&mut collection, free, Some(MULTI)).unwrap();
        assert_eq!(collection.get(free).unwrap().parts, collection.get(MULTI).unwrap().parts);
        assert!(editor.add_multi(&mut collection, free, None).is_err());
        assert!(editor.add_multi(&mut collection, MAX_MULTI_COLLECTION_ID, None).is_err());
        editor.remove_multi(&mut collection, MULTI);
        assert!(collection.get(MULTI).is_none());
        let changed = editor.edits.changes_since_opened(&collection);
        assert_eq!(changed.keys().copied().collect::<Vec<_>>(), [free, MULTI]);
        editor.edits.undo(&mut collection);
        assert!(collection.get(MULTI).is_some());
    }

    #[test]
    fn string_ids_parse_from_text() {
        assert_eq!(parse_string_ids("1, 2 3,,").unwrap(), [1, 2, 3]);
        assert!(parse_string_ids("1, x").is_err());
        assert_eq!(format_string_ids(&[4, 5]), "4, 5");
    }
}
