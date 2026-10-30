//! Undo history and change tracking of the record editors (multis, tileart, waypoints). The edited records stay in
//! their usual store (the loaded `MultiCollection`, the tileart rows, a waypoint package), seen through
//! [`KeyedStore`]; an undo step keeps only the previous values of the records it changed, so large stores are not
//! copied per edit.

use std::collections::{BTreeMap, VecDeque};

/// Undo steps kept per editor; older ones are dropped.
pub const EDIT_UNDO_DEPTH: usize = 100;

/// Records by key: `None` is a missing record.
pub trait KeyedValues<K, V> {
    /// A copy of the record `key`.
    fn value(&self, key: &K) -> Option<V>;
}

/// Editable records by key.
pub trait KeyedStore<K, V>: KeyedValues<K, V> {
    /// Stores, replaces (`Some`) or removes (`None`) the record `key`.
    fn set_value(&mut self, key: &K, value: Option<V>);
}

/// The previous values of the records one step changed.
type EditStep<K, V> = Vec<(K, Option<V>)>;

/// Undo/redo stacks and the opened and saved values of every record edited, for one store.
#[derive(Debug)]
pub struct KeyedEdits<K, V, F> {
    undo: VecDeque<EditStep<K, V>>,
    redo: Vec<EditStep<K, V>>,
    /// Record and field of the last edit: a following edit of the same field joins its undo step.
    last_field: Option<(K, F)>,
    /// Value when the store was opened, of every record edited since.
    opened: BTreeMap<K, Option<V>>,
    /// Value at the last save, of every record edited since.
    saved: BTreeMap<K, Option<V>>,
    dirty: bool,
    /// Increased on every change of the store.
    revision: u64,
}

impl<K, V, F> Default for KeyedEdits<K, V, F> {
    fn default() -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            last_field: None,
            opened: BTreeMap::new(),
            saved: BTreeMap::new(),
            dirty: false,
            revision: 0,
        }
    }
}

impl<K: Ord + Clone, V: Clone + PartialEq, F: Clone + PartialEq> KeyedEdits<K, V, F> {
    /// Edits the record `key` of `store`; when the edit changes it, the previous value goes on the undo stack.
    /// Consecutive edits of the same `field` of the same record (typing, dragging a value) form one undo step.
    pub fn edit<S: KeyedStore<K, V>, R>(
        &mut self,
        store: &mut S,
        key: K,
        field: Option<F>,
        edit: impl FnOnce(&mut Option<V>) -> R,
    ) -> R {
        let before = store.value(&key);
        let mut after = before.clone();
        let result = edit(&mut after);
        if after == before {
            return result;
        }
        self.redo.clear();
        let field = field.map(|field| (key.clone(), field));
        if field.is_none() || field != self.last_field {
            push_bounded(&mut self.undo, vec![(key.clone(), before.clone())]);
        }
        self.last_field = field;
        self.remember(&key, before);
        store.set_value(&key, after);
        self.changed(store);
        result
    }

    /// Ends the current coalesced edit: the next edit is a new undo step.
    pub fn break_coalescing(&mut self) {
        self.last_field = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Undoes the last step; returns the keys it changed (`None` when there is nothing to undo).
    pub fn undo<S: KeyedStore<K, V>>(&mut self, store: &mut S) -> Option<Vec<K>> {
        let step = self.undo.pop_back()?;
        let (keys, inverse) = self.apply(store, step);
        self.redo.push(inverse);
        Some(keys)
    }

    /// Redoes the last undone step; returns the keys it changed (`None` when there is nothing to redo).
    pub fn redo<S: KeyedStore<K, V>>(&mut self, store: &mut S) -> Option<Vec<K>> {
        let step = self.redo.pop()?;
        let (keys, inverse) = self.apply(store, step);
        push_bounded(&mut self.undo, inverse);
        Some(keys)
    }

    /// Puts the records back as opened, as one undoable step; false when nothing differs.
    pub fn revert_to_opened<S: KeyedStore<K, V>>(&mut self, store: &mut S) -> bool {
        let step: EditStep<K, V> = self.changes_since_opened(store).into_iter().collect();
        if step.is_empty() {
            return false;
        }
        let opened: EditStep<K, V> = step.iter().map(|(key, _)| (key.clone(), self.opened[key].clone())).collect();
        let (_, inverse) = self.apply(store, opened);
        self.redo.clear();
        push_bounded(&mut self.undo, inverse);
        true
    }

    /// True when a record differs from its last saved (or opened) value.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The records that differ from the opened store, with their current value (`None`: removed).
    pub fn changes_since_opened<S: KeyedValues<K, V> + ?Sized>(&self, store: &S) -> BTreeMap<K, Option<V>> {
        self.opened
            .iter()
            .filter_map(|(key, opened)| {
                let current = store.value(key);
                (current != *opened).then(|| (key.clone(), current))
            })
            .collect()
    }

    /// Records the current values as saved.
    pub fn mark_saved(&mut self) {
        self.saved.clear();
        self.dirty = false;
    }

    /// Sets the records of `step`; returns their keys and the step that sets them back.
    fn apply<S: KeyedStore<K, V>>(&mut self, store: &mut S, step: EditStep<K, V>) -> (Vec<K>, EditStep<K, V>) {
        let mut keys = Vec::with_capacity(step.len());
        let mut inverse = Vec::with_capacity(step.len());
        for (key, value) in step.into_iter().rev() {
            let current = store.value(&key);
            self.remember(&key, current.clone());
            store.set_value(&key, value);
            keys.push(key.clone());
            inverse.push((key, current));
        }
        inverse.reverse();
        self.last_field = None;
        self.changed(store);
        (keys, inverse)
    }

    fn remember(&mut self, key: &K, value: Option<V>) {
        self.opened.entry(key.clone()).or_insert_with(|| value.clone());
        self.saved.entry(key.clone()).or_insert(value);
    }

    fn changed<S: KeyedValues<K, V> + ?Sized>(&mut self, store: &S) {
        self.revision += 1;
        self.dirty = self.saved.iter().any(|(key, saved)| store.value(key) != *saved);
    }
}

fn push_bounded<T>(stack: &mut VecDeque<T>, step: T) {
    stack.push_back(step);
    while stack.len() > EDIT_UNDO_DEPTH {
        stack.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl KeyedValues<u32, String> for BTreeMap<u32, String> {
        fn value(&self, key: &u32) -> Option<String> {
            self.get(key).cloned()
        }
    }

    impl KeyedStore<u32, String> for BTreeMap<u32, String> {
        fn set_value(&mut self, key: &u32, value: Option<String>) {
            match value {
                Some(value) => self.insert(*key, value),
                None => self.remove(key),
            };
        }
    }

    const FIELD: u8 = 0;
    const OTHER_FIELD: u8 = 1;

    fn store() -> BTreeMap<u32, String> {
        BTreeMap::from([(1, "one".to_string()), (2, "two".to_string())])
    }

    #[test]
    fn edits_undo_redo_and_track_changes() {
        let mut store = store();
        let mut edits = KeyedEdits::<u32, String, u8>::default();
        edits.edit(&mut store, 1, Some(FIELD), |value| value.as_mut().unwrap().push('!'));
        edits.edit(&mut store, 1, Some(FIELD), |value| value.as_mut().unwrap().push('!'));
        edits.edit(&mut store, 3, None, |value| *value = Some("three".to_string()));
        edits.edit(&mut store, 2, None, |value| *value = None);
        assert!(edits.is_dirty());
        assert_eq!(
            edits.changes_since_opened(&store),
            BTreeMap::from([(1, Some("one!!".to_string())), (2, None), (3, Some("three".to_string()))])
        );

        assert_eq!(edits.undo(&mut store), Some(vec![2]));
        assert_eq!(edits.undo(&mut store), Some(vec![3]));
        assert_eq!(edits.undo(&mut store), Some(vec![1]), "the two typed characters are one step");
        assert_eq!(store, self::store());
        assert!(!edits.is_dirty());
        assert_eq!(edits.undo(&mut store), None);
        assert_eq!(edits.redo(&mut store), Some(vec![1]));
        assert_eq!(store[&1], "one!!");
    }

    #[test]
    fn unchanged_edits_are_not_recorded_and_other_fields_split_steps() {
        let mut store = store();
        let mut edits = KeyedEdits::<u32, String, u8>::default();
        edits.edit(&mut store, 1, Some(FIELD), |_| ());
        assert!(!edits.can_undo());
        edits.edit(&mut store, 1, Some(FIELD), |value| *value = Some("a".to_string()));
        edits.edit(&mut store, 1, Some(OTHER_FIELD), |value| *value = Some("b".to_string()));
        edits.undo(&mut store);
        assert_eq!(store[&1], "a");
    }

    #[test]
    fn saving_and_reverting() {
        let mut store = store();
        let mut edits = KeyedEdits::<u32, String, u8>::default();
        edits.edit(&mut store, 1, None, |value| *value = Some("edited".to_string()));
        edits.mark_saved();
        assert!(!edits.is_dirty());
        edits.undo(&mut store);
        assert!(edits.is_dirty(), "undoing a saved edit makes the store differ from the saved state");
        edits.redo(&mut store);
        assert!(!edits.is_dirty());

        assert!(edits.revert_to_opened(&mut store));
        assert_eq!(store, self::store());
        assert!(!edits.revert_to_opened(&mut store));
        edits.undo(&mut store);
        assert_eq!(store[&1], "edited", "reverting is undoable");
    }

    #[test]
    fn the_history_is_bounded() {
        let mut store = store();
        let mut edits = KeyedEdits::<u32, String, u8>::default();
        for index in 0..=EDIT_UNDO_DEPTH {
            edits.edit(&mut store, 1, None, |value| *value = Some(index.to_string()));
        }
        let mut undone = 0;
        while edits.undo(&mut store).is_some() {
            undone += 1;
        }
        assert_eq!(undone, EDIT_UNDO_DEPTH);
        assert_eq!(store[&1], "0");
    }
}
