//! Editing model of the tileart.uop view of the Tile Metadata tab: edits go straight into the loaded records (and
//! their table rows), keyed by entry hash, with undo per record and a save that writes a copy of the client
//! package with only the changed records. No egui here; `ui/tileart_editor.rs` draws it.

use std::path::Path;

use uocf::enhanced::string_dictionary::UoStringDictionary;
use uocf::enhanced::tileart::{tileart_entry_hash, write_tileart_package, TileArtChanges, TileArtEntry};

use crate::app::{collect_tileart_display_rows, TileArtDisplayRow, TileArtFileEntry};
use crate::logic::keyed_edits::{KeyedEdits, KeyedStore, KeyedValues};

/// A record field, for undo coalescing (consecutive edits of one field form one undo step).
pub type TileArtField = &'static str;

/// The loaded records and their table rows (same order, sorted by stored tile id), keyed by entry hash.
pub struct TileArtStore<'a> {
    pub entries: &'a mut Vec<TileArtFileEntry>,
    pub rows: &'a mut Vec<TileArtDisplayRow>,
    pub dictionary: Option<&'a UoStringDictionary>,
}

impl KeyedValues<u64, TileArtEntry> for [TileArtFileEntry] {
    fn value(&self, key: &u64) -> Option<TileArtEntry> {
        self.iter().find(|file| file.filename_hash == *key).map(|file| file.entry.clone())
    }
}

impl KeyedValues<u64, TileArtEntry> for TileArtStore<'_> {
    fn value(&self, key: &u64) -> Option<TileArtEntry> {
        self.entries.value(key)
    }
}

impl KeyedStore<u64, TileArtEntry> for TileArtStore<'_> {
    fn set_value(&mut self, key: &u64, value: Option<TileArtEntry>) {
        if let Some(index) = self.entries.iter().position(|file| file.filename_hash == *key) {
            self.entries.remove(index);
            self.rows.remove(index);
        }
        let Some(entry) = value else {
            return;
        };
        let file = TileArtFileEntry { filename_hash: *key, entry };
        let index = self.entries.partition_point(|existing| existing.entry.tile_id <= file.entry.tile_id);
        let row = collect_tileart_display_rows(std::slice::from_ref(&file), self.dictionary).remove(0);
        self.entries.insert(index, file);
        self.rows.insert(index, row);
    }
}

/// State of the tileart editor.
#[derive(Debug, Default)]
pub struct TileArtEditor {
    /// The editing controls are shown.
    pub enabled: bool,
    pub edits: KeyedEdits<u64, TileArtEntry, TileArtField>,
    pub error: Option<String>,
    /// Tile id typed for a duplicated record.
    pub new_tile_id: u32,
}

impl TileArtEditor {
    /// Edits `field` of the record of entry `hash`.
    pub fn edit(&mut self, store: &mut TileArtStore, hash: u64, field: TileArtField, edit: impl FnOnce(&mut TileArtEntry)) {
        self.edits.edit(store, hash, Some(field), |entry| {
            if let Some(entry) = entry.as_mut() {
                edit(entry);
                entry.sync_counts();
            }
        });
    }

    /// Adds a copy of the record of entry `hash` as tile `tile_id` (entry `build/tileart/%08d.bin`, the record's
    /// stored tile id set to it); returns the new entry hash.
    pub fn duplicate(&mut self, store: &mut TileArtStore, hash: u64, tile_id: u32) -> Result<u64, String> {
        let new_hash = tileart_entry_hash(tile_id);
        if store.value(&new_hash).is_some() {
            return Err(format!("tile {tile_id} already has a record"));
        }
        let mut entry = store.value(&hash).ok_or("the record to duplicate is missing")?;
        entry.tile_id = tile_id;
        self.edits.edit(store, new_hash, None, |slot| *slot = Some(entry));
        Ok(new_hash)
    }

    /// Removes the record of entry `hash`.
    pub fn remove(&mut self, store: &mut TileArtStore, hash: u64) {
        self.edits.edit(store, hash, None, |entry| *entry = None);
    }
}

/// Writes a copy of the client `source` tileart.uop to `output` with the records changed since it was opened
/// (`write_tileart_package`: every other record keeps its bytes).
pub fn save_tileart(
    edits: &KeyedEdits<u64, TileArtEntry, TileArtField>,
    entries: &[TileArtFileEntry],
    source: &Path,
    output: &Path,
) -> Result<TileArtChanges, String> {
    let (mut records, mut removed) = (Vec::new(), Vec::new());
    for (hash, entry) in edits.changes_since_opened(entries) {
        match entry {
            Some(entry) => records.push((hash, entry)),
            None => removed.push(hash),
        }
    }
    write_tileart_package(source, output, &records, &removed).map_err(|error| format!("{error:#}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST_TILE: u32 = 5;
    const SECOND_TILE: u32 = 9;
    const NEW_TILE: u32 = 7;

    fn entries() -> Vec<TileArtFileEntry> {
        [FIRST_TILE, SECOND_TILE]
            .map(|tile_id| TileArtFileEntry {
                filename_hash: tileart_entry_hash(tile_id),
                entry: TileArtEntry { tile_id, ..TileArtEntry::default() },
            })
            .to_vec()
    }

    fn tile_ids(entries: &[TileArtFileEntry]) -> Vec<u32> {
        entries.iter().map(|file| file.entry.tile_id).collect()
    }

    #[test]
    fn edits_keep_entries_and_rows_in_tile_order() {
        let mut entries = entries();
        let mut rows = collect_tileart_display_rows(&entries, None);
        let mut editor = TileArtEditor::default();
        let first = tileart_entry_hash(FIRST_TILE);
        let mut store = TileArtStore { entries: &mut entries, rows: &mut rows, dictionary: None };

        let new_hash = editor.duplicate(&mut store, first, NEW_TILE).unwrap();
        assert_eq!(new_hash, tileart_entry_hash(NEW_TILE));
        assert!(editor.duplicate(&mut store, first, NEW_TILE).is_err());
        assert_eq!(tile_ids(store.entries), [FIRST_TILE, NEW_TILE, SECOND_TILE]);
        assert_eq!(store.rows[1].tile_id, NEW_TILE.to_string());

        editor.edit(&mut store, first, "body_type", |entry| entry.body_type = 666);
        assert_eq!(store.rows[0].body_type, "666");
        editor.remove(&mut store, tileart_entry_hash(SECOND_TILE));
        assert_eq!(tile_ids(store.entries), [FIRST_TILE, NEW_TILE]);
        assert_eq!(store.rows.len(), store.entries.len());

        let changes = editor.edits.changes_since_opened(&store);
        assert_eq!(changes.len(), 3);
        assert!(changes[&tileart_entry_hash(SECOND_TILE)].is_none());
        while editor.edits.undo(&mut store).is_some() {}
        assert_eq!(tile_ids(store.entries), [FIRST_TILE, SECOND_TILE]);
        assert_eq!(store.rows[0].body_type, "0");
    }

    #[test]
    fn edits_keep_the_counts_in_step_with_the_vectors() {
        let mut entries = entries();
        let mut rows = collect_tileart_display_rows(&entries, None);
        let mut editor = TileArtEditor::default();
        let mut store = TileArtStore { entries: &mut entries, rows: &mut rows, dictionary: None };
        let first = tileart_entry_hash(FIRST_TILE);
        editor.edit(&mut store, first, "props1", |entry| entry.prop_vector1.push(Default::default()));
        assert_eq!(store.value(&first).unwrap().prop_count1, 1);
    }
}
