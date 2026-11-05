//! Editing model of the Effects panel: the edited `EffectDefinitionCollection.uop` and the pending
//! `EffectTexture.uop` file changes with one bounded undo history, the selection, the browse filters, the edit
//! operations, the packet id mapping and the collection file operations. No egui here; `ui/effects.rs` draws
//! it. Field meanings are in `uocf::enhanced::effect_definition`.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use uocf::enhanced::effect_definition::{
    DirectionOffset, EarthquakeEffect, EffectCollectionChanges, EffectCollectionEntry, EffectDefinition,
    EffectDefinitionCollection, EffectEntryKey, EffectIssue, EffectMapEntry, LodEffect, PacketEffectOutcome,
    PacketEffectPath, ParticleEffect, PositionOffsetEffect, PropertyFlagEffect, PropertyFlagMode, RandomEffect,
    RandomEffectEntry, RoundRobinEffect, SoundEffect, UserPreferenceEffect, WaitEffect, EFFECT_ID_LIGHTNING,
    EXPLOSION_ITEM_ID, EXPLOSION_TICKS, POSITION_OFFSET_DIRECTION_COUNT, RANDOM_WEIGHT_TOTAL,
};
use uocf::enhanced::string_dictionary::UoStringDictionary;

/// Undo snapshots kept per document; older ones are dropped.
pub const EFFECTS_UNDO_DEPTH: usize = 100;
/// Client defaults of a new wait definition (`UOWaitCollectionDefinition`).
pub const NEW_WAIT_MIN_DELAY_MS: u32 = 2000;
pub const NEW_WAIT_MAX_DELAY_MS: u32 = 4000;
/// Attach mode of every collection particle in the client data.
pub const NEW_PARTICLE_ATTACH_MODE: u8 = 1;
pub const NEW_PARTICLE_SCALE: f32 = 1.0;
/// The `value_08` of every client sound definition.
pub const NEW_SOUND_VALUE_08: f32 = 1.0;
/// Sound id of a new sound definition.
pub const NEW_SOUND_ID: u32 = 0;
/// Property of a new property flag definition: 7 selects by object hue.
pub const NEW_PROPERTY_FLAG_PROPERTY: u32 = 7;
/// A new earthquake: a small, short shake.
pub const NEW_EARTHQUAKE_AMPLITUDE: f64 = 0.1;
pub const NEW_EARTHQUAKE_DURATION_S: f64 = 1.0;
pub const NEW_EARTHQUAKE_FREQUENCY_HZ: f64 = 10.0;
/// First key of a new LOD level (UI Low).
pub const FIRST_LOD_LEVEL: u32 = 1;
/// The client's string dictionary index `n` is the repo dictionary's string `n - 1` (its header swallows the
/// client's empty string 0); see `ParticleEffect::name`.
pub const STRING_DICTIONARY_INDEX_SHIFT: u32 = 1;
/// Marker of a Chaos particle name: the client plays a name containing `.ems` with `UOParticleEffectChaos`.
pub const CHAOS_NAME_MARKER: &str = ".ems";

/// Bounded snapshot stack.
#[derive(Debug)]
pub struct UndoHistory<T> {
    undo: VecDeque<T>,
    redo: Vec<T>,
    last_key: Option<EffectsEditKey>,
}

impl<T> Default for UndoHistory<T> {
    fn default() -> Self {
        Self { undo: VecDeque::new(), redo: Vec::new(), last_key: None }
    }
}

impl<T> UndoHistory<T> {
    /// Records the state before an edit. An edit with the same key as the previous one joins its undo step.
    pub fn record(&mut self, before: T, key: Option<EffectsEditKey>) {
        self.redo.clear();
        if key.is_some() && key == self.last_key {
            return;
        }
        self.last_key = key;
        push_bounded(&mut self.undo, before);
    }

    pub fn break_coalescing(&mut self) {
        self.last_key = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self, current: &mut T) -> bool {
        let Some(previous) = self.undo.pop_back() else {
            return false;
        };
        self.redo.push(std::mem::replace(current, previous));
        self.last_key = None;
        true
    }

    pub fn redo(&mut self, current: &mut T) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        push_bounded(&mut self.undo, std::mem::replace(current, next));
        self.last_key = None;
        true
    }
}

fn push_bounded<T>(stack: &mut VecDeque<T>, snapshot: T) {
    stack.push_back(snapshot);
    while stack.len() > EFFECTS_UNDO_DEPTH {
        stack.pop_front();
    }
}

/// A pending change of an `EffectTexture.uop` file.
#[derive(Debug, Clone, PartialEq)]
pub struct FileOverride {
    /// The file name the entry is stored under (`data/effects/` + lowercase name).
    pub name: String,
    /// The new payload; `None` removes the file.
    pub payload: Option<Arc<[u8]>>,
}

/// An edited NIF that does not write (a list length disagrees with its count field): kept until an edit
/// makes it writable again, when it becomes the file's payload. The file's payload stays the last version that
/// wrote; saving refuses while a draft exists.
#[derive(Debug, Clone, PartialEq)]
pub struct NifDraft {
    /// The file name, as in [`FileOverride::name`].
    pub name: String,
    pub nif: Arc<gamebryo_nif::NifFile>,
    /// Why it does not write.
    pub error: String,
}

/// Everything the panel edits: the collection, the pending file changes and the unwritable NIF drafts, by
/// name hash.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EffectsData {
    pub collection: EffectDefinitionCollection,
    pub files: BTreeMap<u64, FileOverride>,
    pub nif_drafts: BTreeMap<u64, NifDraft>,
}

impl EffectsData {
    /// Stores a file change, dropping the NIF draft of the file.
    pub fn set_file(&mut self, hash: u64, change: FileOverride) {
        self.nif_drafts.remove(&hash);
        self.files.insert(hash, change);
    }

    /// Drops the file change and NIF draft of a file.
    pub fn restore_file(&mut self, hash: u64) {
        self.nif_drafts.remove(&hash);
        self.files.remove(&hash);
    }
}

/// Where the edited collection was opened from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionSource {
    /// Nothing opened yet (an empty collection).
    None,
    /// The loaded client's `EffectDefinitionCollection.uop`.
    ClientPackage(PathBuf),
    /// Another `EffectDefinitionCollection.uop`.
    Package(PathBuf),
    /// A KDL text file.
    Kdl(PathBuf),
}

impl CollectionSource {
    pub fn label(&self) -> String {
        match self {
            Self::None => "no collection".to_string(),
            Self::ClientPackage(path) => format!("client {}", path.display()),
            Self::Package(path) => format!("package {}", path.display()),
            Self::Kdl(path) => format!("KDL {}", path.display()),
        }
    }

    /// The package a saved collection is a copy of, when the source is one.
    pub fn package_path(&self) -> Option<&Path> {
        match self {
            Self::ClientPackage(path) | Self::Package(path) => Some(path),
            Self::None | Self::Kdl(_) => None,
        }
    }
}

/// Identifies an edit for undo coalescing: consecutive edits with the same key form one undo step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectsEditKey {
    /// A field of a definition (`field` names it).
    Field { entry: usize, path: Vec<PathStep>, field: &'static str },
    /// The text of a script file.
    FileText { hash: u64 },
    /// A field of a NIF file block (`path` names the field inside the block).
    NifField { hash: u64, block: usize, path: String },
}

/// The edited data, its opened and saved states and the undo history.
#[derive(Debug)]
pub struct EffectsDocument {
    data: EffectsData,
    opened: EffectsData,
    saved_collection: EffectDefinitionCollection,
    saved_files: BTreeMap<u64, FileOverride>,
    source: CollectionSource,
    history: UndoHistory<EffectsData>,
    /// Increased on every change of `data`, for the caches of the panel.
    revision: u64,
}

impl Default for EffectsDocument {
    fn default() -> Self {
        Self::new(EffectDefinitionCollection::default(), CollectionSource::None)
    }
}

impl EffectsDocument {
    pub fn new(collection: EffectDefinitionCollection, source: CollectionSource) -> Self {
        let data = EffectsData { collection: collection.clone(), files: BTreeMap::new(), nif_drafts: BTreeMap::new() };
        Self {
            opened: data.clone(),
            saved_collection: collection,
            saved_files: BTreeMap::new(),
            data,
            source,
            history: UndoHistory::default(),
            revision: 0,
        }
    }

    pub fn collection(&self) -> &EffectDefinitionCollection {
        &self.data.collection
    }

    pub fn files(&self) -> &BTreeMap<u64, FileOverride> {
        &self.data.files
    }

    pub fn source(&self) -> &CollectionSource {
        &self.source
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The collection differs from the last opened or saved one.
    pub fn collection_dirty(&self) -> bool {
        self.data.collection != self.saved_collection
    }

    /// The pending file changes differ from the last saved ones, or a NIF draft is pending.
    pub fn files_dirty(&self) -> bool {
        self.data.files != self.saved_files || !self.data.nif_drafts.is_empty()
    }

    /// The edited NIFs that do not write.
    pub fn nif_drafts(&self) -> &BTreeMap<u64, NifDraft> {
        &self.data.nif_drafts
    }

    pub fn is_dirty(&self) -> bool {
        self.collection_dirty() || self.files_dirty()
    }

    /// The data differs from the opened state ("revert to loaded" has something to do).
    pub fn differs_from_opened(&self) -> bool {
        self.data != self.opened
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Applies an edit; when it changes the data, the previous state goes on the undo stack.
    pub fn edit<R>(&mut self, key: Option<EffectsEditKey>, edit: impl FnOnce(&mut EffectsData) -> R) -> R {
        let before = self.data.clone();
        let result = edit(&mut self.data);
        if self.data != before {
            self.history.record(before, key);
            self.revision += 1;
        }
        result
    }

    /// [`Self::edit`] of the collection only.
    pub fn edit_collection<R>(
        &mut self,
        key: Option<EffectsEditKey>,
        edit: impl FnOnce(&mut EffectDefinitionCollection) -> R,
    ) -> R {
        self.edit(key, |data| edit(&mut data.collection))
    }

    pub fn break_coalescing(&mut self) {
        self.history.break_coalescing();
    }

    pub fn undo(&mut self) -> bool {
        let undone = self.history.undo(&mut self.data);
        if undone {
            self.revision += 1;
        }
        undone
    }

    pub fn redo(&mut self) -> bool {
        let redone = self.history.redo(&mut self.data);
        if redone {
            self.revision += 1;
        }
        redone
    }

    /// Goes back to the opened data, as one undoable edit.
    pub fn revert_to_opened(&mut self) {
        let opened = self.opened.clone();
        self.edit(None, |data| *data = opened);
    }

    /// Replaces the collection with a newly opened one; the undo history restarts (file changes are kept).
    pub fn replace_collection(&mut self, collection: EffectDefinitionCollection, source: CollectionSource) {
        self.data.collection = collection.clone();
        self.opened.collection = collection.clone();
        self.saved_collection = collection;
        self.source = source;
        self.history = UndoHistory::default();
        self.revision += 1;
    }

    /// Drops every pending file change (another `EffectTexture.uop` was opened); the undo history restarts.
    pub fn reset_files(&mut self) {
        self.data.files.clear();
        self.data.nif_drafts.clear();
        self.opened.files.clear();
        self.opened.nif_drafts.clear();
        self.saved_files.clear();
        self.history = UndoHistory::default();
        self.revision += 1;
    }

    pub fn mark_collection_saved(&mut self) {
        self.saved_collection = self.data.collection.clone();
    }

    pub fn mark_files_saved(&mut self) {
        self.saved_files = self.data.files.clone();
    }
}

/// A child list of a definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChildList {
    /// The top-level definitions of an entry (or of a tileart record).
    Top,
    /// The children of a round-robin, wait or user preference definition.
    Children,
    /// The list of LOD level `index` (position in `levels`).
    Level(usize),
    /// The default children of a property flag definition.
    Default,
    /// The list of property flag case `index` (position in `cases`).
    Case(usize),
    /// The list of random entry `index`.
    RandomEntry(usize),
}

/// One step of a definition path: a position in a child list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PathStep {
    pub list: ChildList,
    pub index: usize,
}

impl PathStep {
    pub fn top(index: usize) -> Self {
        Self { list: ChildList::Top, index }
    }
}

/// The child lists of a definition, in display order.
pub fn child_lists(definition: &EffectDefinition) -> Vec<(ChildList, &[EffectDefinition])> {
    match definition {
        EffectDefinition::Particle(_)
        | EffectDefinition::Sound(_)
        | EffectDefinition::PositionOffset(_)
        | EffectDefinition::Earthquake(_) => Vec::new(),
        EffectDefinition::RoundRobin(effect) => vec![(ChildList::Children, effect.children.as_slice())],
        EffectDefinition::Wait(effect) => vec![(ChildList::Children, effect.children.as_slice())],
        EffectDefinition::UserPreference(effect) => vec![(ChildList::Children, effect.children.as_slice())],
        EffectDefinition::Lod(effect) => effect
            .levels
            .iter()
            .enumerate()
            .map(|(index, level)| (ChildList::Level(index), level.children.as_slice()))
            .collect(),
        EffectDefinition::PropertyFlag(effect) => std::iter::once((ChildList::Default, effect.default_children.as_slice()))
            .chain(effect.cases.iter().enumerate().map(|(index, case)| (ChildList::Case(index), case.children.as_slice())))
            .collect(),
        EffectDefinition::Random(effect) => effect
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (ChildList::RandomEntry(index), entry.children.as_slice()))
            .collect(),
    }
}

fn child_list_mut(definition: &mut EffectDefinition, list: ChildList) -> Option<&mut Vec<EffectDefinition>> {
    match (definition, list) {
        (EffectDefinition::RoundRobin(effect), ChildList::Children) => Some(&mut effect.children),
        (EffectDefinition::Wait(effect), ChildList::Children) => Some(&mut effect.children),
        (EffectDefinition::UserPreference(effect), ChildList::Children) => Some(&mut effect.children),
        (EffectDefinition::Lod(effect), ChildList::Level(index)) => effect.levels.get_mut(index).map(|level| &mut level.children),
        (EffectDefinition::PropertyFlag(effect), ChildList::Default) => Some(&mut effect.default_children),
        (EffectDefinition::PropertyFlag(effect), ChildList::Case(index)) => {
            effect.cases.get_mut(index).map(|case| &mut case.children)
        }
        (EffectDefinition::Random(effect), ChildList::RandomEntry(index)) => {
            effect.entries.get_mut(index).map(|entry| &mut entry.children)
        }
        _ => None,
    }
}

fn child_list_of(definition: &EffectDefinition, list: ChildList) -> Option<&[EffectDefinition]> {
    child_lists(definition).into_iter().find(|(candidate, _)| *candidate == list).map(|(_, children)| children)
}

/// The label of a child list (`level 3`, `case 33`, `entry 2 (40%)`, ...).
pub fn child_list_label(definition: &EffectDefinition, list: ChildList) -> String {
    match (definition, list) {
        (_, ChildList::Top) => "definitions".to_string(),
        (_, ChildList::Children) => "children".to_string(),
        (_, ChildList::Default) => "default".to_string(),
        (EffectDefinition::Lod(effect), ChildList::Level(index)) => {
            effect.levels.get(index).map_or_else(|| "level ?".to_string(), |level| format!("level {}", level.key))
        }
        (EffectDefinition::PropertyFlag(effect), ChildList::Case(index)) => {
            effect.cases.get(index).map_or_else(|| "case ?".to_string(), |case| format!("case {}", case.key))
        }
        (EffectDefinition::Random(effect), ChildList::RandomEntry(index)) => effect
            .entries
            .get(index)
            .map_or_else(|| "entry ?".to_string(), |entry| format!("entry {index} ({}%)", entry.weight)),
        _ => "?".to_string(),
    }
}

/// The definition at `path` in a top-level list.
pub fn definition_at<'a>(definitions: &'a [EffectDefinition], path: &[PathStep]) -> Option<&'a EffectDefinition> {
    let (first, rest) = path.split_first()?;
    let mut current = definitions.get(first.index)?;
    for step in rest {
        current = child_list_of(current, step.list)?.get(step.index)?;
    }
    Some(current)
}

pub fn definition_at_mut<'a>(
    definitions: &'a mut [EffectDefinition],
    path: &[PathStep],
) -> Option<&'a mut EffectDefinition> {
    let (first, rest) = path.split_first()?;
    let mut current = definitions.get_mut(first.index)?;
    for step in rest {
        current = child_list_mut(current, step.list)?.get_mut(step.index)?;
    }
    Some(current)
}

/// The list holding the definition at `path` (the top-level list for a one-step path).
fn list_holding_mut<'a>(
    definitions: &'a mut Vec<EffectDefinition>,
    path: &[PathStep],
) -> Option<&'a mut Vec<EffectDefinition>> {
    let (last, parent) = path.split_last()?;
    if parent.is_empty() {
        return Some(definitions);
    }
    child_list_mut(definition_at_mut(definitions, parent)?, last.list)
}

/// The kinds of definitions, one per client type key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefinitionKind {
    Particle,
    Sound,
    RoundRobin,
    PositionOffset,
    Wait,
    Lod,
    PropertyFlag,
    Earthquake,
    Random,
    UserPreference,
}

impl DefinitionKind {
    pub const ALL: [Self; 10] = [
        Self::Particle,
        Self::Sound,
        Self::RoundRobin,
        Self::PositionOffset,
        Self::Wait,
        Self::Lod,
        Self::PropertyFlag,
        Self::Earthquake,
        Self::Random,
        Self::UserPreference,
    ];

    pub fn of(definition: &EffectDefinition) -> Self {
        match definition {
            EffectDefinition::Particle(_) => Self::Particle,
            EffectDefinition::Sound(_) => Self::Sound,
            EffectDefinition::RoundRobin(_) => Self::RoundRobin,
            EffectDefinition::PositionOffset(_) => Self::PositionOffset,
            EffectDefinition::Wait(_) => Self::Wait,
            EffectDefinition::Lod(_) => Self::Lod,
            EffectDefinition::PropertyFlag(_) => Self::PropertyFlag,
            EffectDefinition::Earthquake(_) => Self::Earthquake,
            EffectDefinition::Random(_) => Self::Random,
            EffectDefinition::UserPreference(_) => Self::UserPreference,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Particle => "Particle",
            Self::Sound => "Sound",
            Self::RoundRobin => "Round robin",
            Self::PositionOffset => "Position offset",
            Self::Wait => "Wait",
            Self::Lod => "LOD",
            Self::PropertyFlag => "Property flag",
            Self::Earthquake => "Earthquake",
            Self::Random => "Random",
            Self::UserPreference => "User preference",
        }
    }

    /// What the client does with the definition (from the `effect_definition` docs).
    pub fn description(self) -> &'static str {
        match self {
            Self::Particle => {
                "Key 0: a .nif or .ems particle scene. A name containing .ems plays a Chaos script \
                 (UOParticleEffectChaos), any other name a (hued) Gamebryo NIF scene."
            }
            Self::Sound => "Key 1: a sound played at the effect position, with distance attenuation.",
            Self::RoundRobin => {
                "Key 2: plays one child per use, cycling through them in order. The cycle index belongs to the \
                 definition: it is shared by every object of the same graphic and every play of the entry."
            }
            Self::PositionOffset => {
                "Key 7: per-direction position offsets. The effect object never reads them; the client reads the \
                 definition from a mount's animation sequence, to place the rider. No shipped data uses it."
            }
            Self::Wait => {
                "Key 10: throttle. Plays its children only if the per-definition next time has passed, then sets \
                 the next time to now + min (next multiple of min when aligned) + uniform [0, max - min] ms. It \
                 never fires again by itself."
            }
            Self::Lod => {
                "Key 11: plays the level for the particle LOD setting (UI Low/Medium/High = 1..3, default 3), \
                 falling back to the next lower key; nothing if no key is <= the setting."
            }
            Self::PropertyFlag => {
                "Key 12: selects children by an object property: flag-mask mode looks up (object flags & mask) \
                 != 0 (no match plays nothing); property mode looks up the object value selected by `property` \
                 (no match plays the default children)."
            }
            Self::Earthquake => {
                "Key 15: shakes the camera target x by amplitude * sin(elapsed * frequency * 2pi) for duration \
                 seconds."
            }
            Self::Random => {
                "Key 16: plays one entry picked by weight (percent): the client draws r in [0, 100) and plays the \
                 first entry with r < weight after subtracting the previous weights."
            }
            Self::UserPreference => {
                "Key 17: skips its children when all of the mask bits are set in the user effect settings (bit 0 \
                 = footsteps disabled), or when the mask is 1 and the object value vt+0xa0 is negative."
            }
        }
    }

    /// A new definition with the client defaults and no children.
    pub fn new_definition(self) -> EffectDefinition {
        match self {
            Self::Particle => EffectDefinition::Particle(ParticleEffect {
                name_index: 0,
                offset_x: 0.0,
                offset_y: 0.0,
                camera_offset: 0.0,
                scale: NEW_PARTICLE_SCALE,
                attach_mode: NEW_PARTICLE_ATTACH_MODE,
                duration: 0.0,
                looping: false,
                field_3c: 0,
                hued: false,
            }),
            Self::Sound => EffectDefinition::Sound(SoundEffect { sound_id: NEW_SOUND_ID, value_08: NEW_SOUND_VALUE_08 }),
            Self::RoundRobin => EffectDefinition::RoundRobin(RoundRobinEffect { children: Vec::new() }),
            Self::PositionOffset => EffectDefinition::PositionOffset(PositionOffsetEffect {
                offsets: [DirectionOffset::default(); POSITION_OFFSET_DIRECTION_COUNT],
            }),
            Self::Wait => EffectDefinition::Wait(WaitEffect {
                children: Vec::new(),
                min_delay_ms: NEW_WAIT_MIN_DELAY_MS,
                max_delay_ms: NEW_WAIT_MAX_DELAY_MS,
                aligned: false,
            }),
            Self::Lod => EffectDefinition::Lod(LodEffect { levels: Vec::new() }),
            Self::PropertyFlag => EffectDefinition::PropertyFlag(PropertyFlagEffect {
                mode: PropertyFlagMode::Property,
                mask: 0,
                property: NEW_PROPERTY_FLAG_PROPERTY,
                default_children: Vec::new(),
                cases: Vec::new(),
            }),
            Self::Earthquake => EffectDefinition::Earthquake(EarthquakeEffect {
                amplitude: NEW_EARTHQUAKE_AMPLITUDE,
                duration: NEW_EARTHQUAKE_DURATION_S,
                frequency: NEW_EARTHQUAKE_FREQUENCY_HZ,
            }),
            Self::Random => EffectDefinition::Random(RandomEffect { entries: Vec::new() }),
            Self::UserPreference => {
                EffectDefinition::UserPreference(UserPreferenceEffect { mask: 0, children: Vec::new() })
            }
        }
    }
}

/// The browse list on the left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrowseView {
    #[default]
    Entries,
    TileArt,
    Files,
}

/// What the panel shows on the right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectsSelection {
    /// A collection entry by index.
    Entry { entry: usize },
    /// A definition of a collection entry.
    Definition { entry: usize, path: Vec<PathStep> },
    /// A tileart record with effects (read only); an empty path selects the record.
    TileArt { tile_id: u32, path: Vec<PathStep> },
    /// An `EffectTexture.uop` file by name hash.
    File { hash: u64 },
}

impl EffectsSelection {
    /// The selection if it still exists in `collection`, else the closest existing parent.
    pub fn resolve(self, collection: &EffectDefinitionCollection) -> Option<Self> {
        match self {
            Self::Entry { entry } => (entry < collection.entries.len()).then_some(self),
            Self::Definition { entry, mut path } => {
                let definitions = &collection.entries.get(entry)?.definitions;
                while !path.is_empty() && definition_at(definitions, &path).is_none() {
                    path.pop();
                }
                Some(if path.is_empty() { Self::Entry { entry } } else { Self::Definition { entry, path } })
            }
            Self::TileArt { .. } | Self::File { .. } => Some(self),
        }
    }

    /// The collection entry index of an entry or definition selection.
    pub fn entry(&self) -> Option<usize> {
        match self {
            Self::Entry { entry } | Self::Definition { entry, .. } => Some(*entry),
            Self::TileArt { .. } | Self::File { .. } => None,
        }
    }
}

/// A way the effect packets play a collection id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketRoute {
    /// A value of a 0xC7 field.
    Field { path: PacketEffectPath, value: u16 },
    /// Packet type 1 (lightning) plays `EFFECT_ID_LIGHTNING` whatever the fields.
    Lightning,
}

impl PacketRoute {
    pub fn label(&self) -> String {
        match self {
            Self::Field { path, value } => format!("{}: {value} (0x{value:04X})", path.label()),
            Self::Lightning => "lightning (type 1)".to_string(),
        }
    }
}

/// Why no packet plays a collection id with no [`packet_routes_for_collection_id`].
pub const PACKET_UNREACHABLE_NOTE: &str = "above the packet range: the 16-bit fields reach ids 0..65534 only";

/// Every packet path and value that plays collection id `id` (the inverse of [`PacketEffectPath::outcome`]): the
/// fixed effect and the explode effect fields play id = value - 1, except that the fixed effect field intercepts
/// 9501 and 9502 (so ids 9500 and 9501 play only as explode effects); the moving effect field plays no entry.
pub fn packet_routes_for_collection_id(id: u32) -> Vec<PacketRoute> {
    let mut routes: Vec<PacketRoute> = PacketEffectPath::ALL
        .into_iter()
        .filter_map(|path| path.packet_value_for_collection(id).map(|value| PacketRoute::Field { path, value }))
        .collect();
    if id == EFFECT_ID_LIGHTNING {
        routes.push(PacketRoute::Lightning);
    }
    routes
}

/// What a value of a packet field plays: the collection id, or what the client does instead. `Err` texts end
/// with the hue the classic object takes.
pub fn describe_packet_effect(path: PacketEffectPath, value: u16) -> Result<u32, String> {
    const OBJECT_HUE_NOTE: &str = "its object takes the packet hue + 1 (0 stays 0), which hued particles of its \
        tileart effects take";
    let tileart = |suppress: bool| if suppress { "without" } else { "with" };
    match path.outcome(value) {
        PacketEffectOutcome::Collection(id) => Ok(id),
        PacketEffectOutcome::Nothing => Err("plays nothing".to_string()),
        PacketEffectOutcome::ClassicTileEffect { suppress_tileart_effects } => Err(format!(
            "plays the classic tile effect of the packet item (if non-zero), {} its tileart effects; {OBJECT_HUE_NOTE}",
            tileart(suppress_tileart_effects)
        )),
        PacketEffectOutcome::Projectile { suppress_tileart_effects } => Err(format!(
            "plays no collection entry: the projectile shows the packet item {} its tileart effects; {OBJECT_HUE_NOTE}",
            tileart(suppress_tileart_effects)
        )),
        PacketEffectOutcome::ClassicExplosion => Err(format!(
            "plays the classic explosion: item 0x{EXPLOSION_ITEM_ID:04X} for {EXPLOSION_TICKS} ticks with the \
             projectile's hue and render mode"
        )),
    }
}

/// Parses an id typed as decimal or `0x` hex; `None` for empty or invalid text.
pub fn parse_id(text: &str) -> Option<u32> {
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

/// The particle file name of a definition, through the string dictionary.
pub fn particle_name(particle: &ParticleEffect, strings: Option<&UoStringDictionary>) -> Option<String> {
    strings.and_then(|strings| particle.name(strings)).map(str::to_owned)
}

/// Whether the client plays a particle name with the Chaos engine.
pub fn is_chaos_name(name: &str) -> bool {
    name.to_ascii_lowercase().contains(CHAOS_NAME_MARKER)
}

/// String dictionary index by lowercase string, to set a particle name by typing it.
#[derive(Debug, Default)]
pub struct NameIndex {
    by_name: HashMap<String, u32>,
}

impl NameIndex {
    pub fn new(strings: &UoStringDictionary) -> Self {
        let mut by_name = HashMap::new();
        for (index, text) in strings.iter() {
            let Some(name_index) = u32::try_from(index).ok().and_then(|index| index.checked_add(STRING_DICTIONARY_INDEX_SHIFT))
            else {
                continue;
            };
            by_name.entry(text.to_ascii_lowercase()).or_insert(name_index);
        }
        Self { by_name }
    }

    /// The client name index of a string (case-insensitive; the first one when repeated).
    pub fn name_index(&self, name: &str) -> Option<u32> {
        self.by_name.get(&name.trim().to_ascii_lowercase()).copied()
    }
}

/// Which particles an entry must have to pass the filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParticleKindFilter {
    #[default]
    Any,
    /// At least one Chaos (`.ems`) particle.
    Chaos,
    /// At least one NIF particle.
    Nif,
}

/// Filters of the entry and tileart lists. Empty fields do not filter.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DefinitionFilter {
    /// An id (decimal or `0x` hex) matches the entry id, its packet value or the tile id; other text matches
    /// particle names and definition kind labels (case insensitive).
    pub text: String,
    /// Keep lists with a definition of this kind, at any depth.
    pub kind: Option<DefinitionKind>,
    pub particle: ParticleKindFilter,
}

impl DefinitionFilter {
    pub fn is_active(&self) -> bool {
        !self.text.trim().is_empty() || self.kind.is_some() || self.particle != ParticleKindFilter::Any
    }

    /// Whether a definition list passes. `ids` are the numbers an id query matches; `texts` extra texts a text
    /// query matches (names, labels).
    pub fn matches(
        &self,
        ids: &[u32],
        texts: &[&str],
        definitions: &[EffectDefinition],
        name_of: &dyn Fn(&ParticleEffect) -> Option<String>,
    ) -> bool {
        let mut kinds = Vec::new();
        let mut names = Vec::new();
        for definition in definitions {
            definition.visit(&mut |definition| {
                kinds.push(DefinitionKind::of(definition));
                if let EffectDefinition::Particle(particle) = definition {
                    names.push(name_of(particle));
                }
            });
        }
        if self.kind.is_some_and(|kind| !kinds.contains(&kind)) {
            return false;
        }
        let particle_matches = |chaos: bool| names.iter().flatten().any(|name| is_chaos_name(name) == chaos);
        match self.particle {
            ParticleKindFilter::Any => {}
            ParticleKindFilter::Chaos if !particle_matches(true) => return false,
            ParticleKindFilter::Nif if !particle_matches(false) => return false,
            ParticleKindFilter::Chaos | ParticleKindFilter::Nif => {}
        }
        let query = self.text.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        if let Some(id) = parse_id(&query) {
            if ids.contains(&id) {
                return true;
            }
        }
        texts.iter().any(|text| text.to_lowercase().contains(&query))
            || names.iter().flatten().any(|name| name.to_lowercase().contains(&query))
            || kinds.iter().any(|kind| kind.label().to_lowercase().contains(&query))
    }

    /// The indices of the collection entries that pass.
    pub fn visible_entries(
        &self,
        collection: &EffectDefinitionCollection,
        name_of: &dyn Fn(&ParticleEffect) -> Option<String>,
    ) -> Vec<usize> {
        collection
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                let mut ids = Vec::new();
                if let Some(id) = entry.key.id() {
                    ids.push(id);
                    for route in packet_routes_for_collection_id(id) {
                        if let PacketRoute::Field { value, .. } = route {
                            ids.push(u32::from(value));
                        }
                    }
                }
                let hash = format!("{:016x}", entry.key.hash());
                self.matches(&ids, &[&hash], &entry.definitions, name_of)
            })
            .map(|(index, _)| index)
            .collect()
    }
}

/// The id after the highest collection id (0 for an empty collection).
pub fn next_free_entry_id(collection: &EffectDefinitionCollection) -> u32 {
    collection.ids().max().map_or(0, |id| id.saturating_add(1))
}

fn ensure_id_free(collection: &EffectDefinitionCollection, id: u32, except: Option<usize>) -> Result<(), String> {
    let hash = EffectEntryKey::Id(id).hash();
    let taken = collection
        .entries
        .iter()
        .enumerate()
        .any(|(index, entry)| Some(index) != except && entry.key.hash() == hash);
    if taken {
        Err(format!("collection id {id} is already used"))
    } else {
        Ok(())
    }
}

/// Adds an empty entry (`id`, or the next free id) at the end.
pub fn add_entry(collection: &mut EffectDefinitionCollection, id: Option<u32>) -> Result<EffectsSelection, String> {
    let id = id.unwrap_or_else(|| next_free_entry_id(collection));
    ensure_id_free(collection, id, None)?;
    collection.entries.push(EffectCollectionEntry { key: EffectEntryKey::Id(id), definitions: Vec::new() });
    Ok(EffectsSelection::Entry { entry: collection.entries.len() - 1 })
}

/// Inserts a copy of an entry with id `new_id` right after it.
pub fn duplicate_entry(
    collection: &mut EffectDefinitionCollection,
    entry: usize,
    new_id: u32,
) -> Result<EffectsSelection, String> {
    let definitions = collection.entries.get(entry).ok_or("no such entry")?.definitions.clone();
    ensure_id_free(collection, new_id, None)?;
    collection.entries.insert(entry + 1, EffectCollectionEntry { key: EffectEntryKey::Id(new_id), definitions });
    Ok(EffectsSelection::Entry { entry: entry + 1 })
}

/// Removes an entry; selects the entry now at its index, the previous one, or nothing.
pub fn remove_entry(collection: &mut EffectDefinitionCollection, entry: usize) -> Option<EffectsSelection> {
    if entry >= collection.entries.len() {
        return None;
    }
    collection.entries.remove(entry);
    let remaining = collection.entries.len();
    (remaining > 0).then(|| EffectsSelection::Entry { entry: entry.min(remaining - 1) })
}

/// Gives an entry another collection id (unique in the collection).
pub fn set_entry_id(collection: &mut EffectDefinitionCollection, entry: usize, id: u32) -> Result<(), String> {
    ensure_id_free(collection, id, Some(entry))?;
    collection.entries.get_mut(entry).ok_or("no such entry")?.key = EffectEntryKey::Id(id);
    Ok(())
}

/// Appends a new definition of `kind` to a list: the entry's top-level list when `parent` is empty, else the
/// `list` of the definition at `parent`. Returns its selection.
pub fn add_definition(
    collection: &mut EffectDefinitionCollection,
    entry: usize,
    parent: &[PathStep],
    list: ChildList,
    kind: DefinitionKind,
) -> Option<EffectsSelection> {
    let definitions = &mut collection.entries.get_mut(entry)?.definitions;
    let target = if parent.is_empty() {
        definitions
    } else {
        child_list_mut(definition_at_mut(definitions, parent)?, list)?
    };
    target.push(kind.new_definition());
    let index = target.len() - 1;
    let mut path = parent.to_vec();
    path.push(if parent.is_empty() { PathStep::top(index) } else { PathStep { list, index } });
    Some(EffectsSelection::Definition { entry, path })
}

/// Removes a definition; selects the sibling now at its index, the previous one, or the parent.
pub fn remove_definition(
    collection: &mut EffectDefinitionCollection,
    entry: usize,
    path: &[PathStep],
) -> Option<EffectsSelection> {
    let list = list_holding_mut(&mut collection.entries.get_mut(entry)?.definitions, path)?;
    let last = *path.last()?;
    if last.index >= list.len() {
        return None;
    }
    list.remove(last.index);
    let remaining = list.len();
    let mut path = path.to_vec();
    if remaining == 0 {
        path.pop();
    } else if let Some(step) = path.last_mut() {
        step.index = last.index.min(remaining - 1);
    }
    Some(if path.is_empty() { EffectsSelection::Entry { entry } } else { EffectsSelection::Definition { entry, path } })
}

/// Inserts a copy of a definition right after it.
pub fn duplicate_definition(
    collection: &mut EffectDefinitionCollection,
    entry: usize,
    path: &[PathStep],
) -> Option<EffectsSelection> {
    let list = list_holding_mut(&mut collection.entries.get_mut(entry)?.definitions, path)?;
    let last = *path.last()?;
    let copy = list.get(last.index)?.clone();
    list.insert(last.index + 1, copy);
    let mut path = path.to_vec();
    path.last_mut()?.index += 1;
    Some(EffectsSelection::Definition { entry, path })
}

/// Moves a definition by `delta` positions within its list (clamped); returns its new selection.
pub fn move_definition(
    collection: &mut EffectDefinitionCollection,
    entry: usize,
    path: &[PathStep],
    delta: isize,
) -> Option<EffectsSelection> {
    let list = list_holding_mut(&mut collection.entries.get_mut(entry)?.definitions, path)?;
    let last = *path.last()?;
    if last.index >= list.len() {
        return None;
    }
    let target = last.index.saturating_add_signed(delta).min(list.len() - 1);
    let moved = list.remove(last.index);
    list.insert(target, moved);
    let mut path = path.to_vec();
    path.last_mut()?.index = target;
    Some(EffectsSelection::Definition { entry, path })
}

/// Adds a keyed list to a definition: a LOD level (the key after the highest, from [`FIRST_LOD_LEVEL`]), a
/// property flag case (the key after the highest, from 0) or a random entry (the weight left to
/// [`RANDOM_WEIGHT_TOTAL`]). Returns the new list, `None` for a definition without keyed lists.
pub fn add_keyed_list(definition: &mut EffectDefinition) -> Option<ChildList> {
    match definition {
        EffectDefinition::Lod(effect) => {
            let key = effect.levels.iter().map(|level| level.key).max().map_or(FIRST_LOD_LEVEL, |key| key.saturating_add(1));
            effect.levels.push(EffectMapEntry { key, children: Vec::new() });
            Some(ChildList::Level(effect.levels.len() - 1))
        }
        EffectDefinition::PropertyFlag(effect) => {
            let key = effect.cases.iter().map(|case| case.key).max().map_or(0, |key| key.saturating_add(1));
            effect.cases.push(EffectMapEntry { key, children: Vec::new() });
            Some(ChildList::Case(effect.cases.len() - 1))
        }
        EffectDefinition::Random(effect) => {
            let used: f32 = effect.entries.iter().map(|entry| entry.weight).sum();
            let weight = (RANDOM_WEIGHT_TOTAL - used).max(0.0);
            effect.entries.push(RandomEffectEntry { weight, children: Vec::new() });
            Some(ChildList::RandomEntry(effect.entries.len() - 1))
        }
        _ => None,
    }
}

/// Removes a keyed list (LOD level, case or random entry) with its children; false when there is none.
pub fn remove_keyed_list(definition: &mut EffectDefinition, list: ChildList) -> bool {
    fn remove<T>(items: &mut Vec<T>, index: usize) -> bool {
        if index < items.len() {
            items.remove(index);
            true
        } else {
            false
        }
    }
    match (definition, list) {
        (EffectDefinition::Lod(effect), ChildList::Level(index)) => remove(&mut effect.levels, index),
        (EffectDefinition::PropertyFlag(effect), ChildList::Case(index)) => remove(&mut effect.cases, index),
        (EffectDefinition::Random(effect), ChildList::RandomEntry(index)) => remove(&mut effect.entries, index),
        _ => false,
    }
}

/// Whether new children can be added to `list` of the definition (it exists).
pub fn accepts_children(definition: &EffectDefinition, list: ChildList) -> bool {
    child_list_of(definition, list).is_some()
}

/// The selection of a collection validation issue: its definition (the issue path), else its entry.
pub fn issue_selection(collection: &EffectDefinitionCollection, issue: &EffectIssue) -> Option<EffectsSelection> {
    let key = issue.entry?;
    let entry = collection.entries.iter().position(|candidate| candidate.key == key)?;
    let path = parse_issue_path(&collection.entries[entry].definitions, &issue.path).unwrap_or_default();
    Some(if path.is_empty() { EffectsSelection::Entry { entry } } else { EffectsSelection::Definition { entry, path } })
}

/// Parses a validation path (`2/entry 1/0`, `0/level 3/1`, `1/default/0`, `0/case 7/0`) into a definition path.
/// A trailing list label without an index selects its definition.
pub fn parse_issue_path(definitions: &[EffectDefinition], text: &str) -> Option<Vec<PathStep>> {
    let mut segments = text.split('/').filter(|segment| !segment.is_empty());
    let first = segments.next()?.trim().parse().ok()?;
    let mut path = vec![PathStep::top(first)];
    let mut pending: Option<ChildList> = None;
    for segment in segments {
        let segment = segment.trim();
        if let Ok(index) = segment.parse::<usize>() {
            path.push(PathStep { list: pending.take().unwrap_or(ChildList::Children), index });
            continue;
        }
        let current = definition_at(definitions, &path)?;
        let (label, value) = segment.split_once(' ').unwrap_or((segment, ""));
        let key = value.trim().parse::<u32>().ok();
        pending = Some(match (label, current) {
            ("default", _) => ChildList::Default,
            ("entry", _) => ChildList::RandomEntry(value.trim().parse().ok()?),
            ("level", EffectDefinition::Lod(effect)) => {
                ChildList::Level(effect.levels.iter().position(|level| Some(level.key) == key)?)
            }
            ("case", EffectDefinition::PropertyFlag(effect)) => {
                ChildList::Case(effect.cases.iter().position(|case| Some(case.key) == key)?)
            }
            _ => return None,
        });
    }
    definition_at(definitions, &path).map(|_| path)
}

/// Who uses an `EffectTexture.uop` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileUser {
    /// A collection entry (index) has a particle with the file's name.
    Entry(usize),
    /// A tileart record (tile id) has a particle with the file's name.
    TileArt(u32),
    /// A Chaos script (name hash) names the file as a texture or mesh, or a NIF scene names it as a texture.
    Script(u64),
}

/// Users of every referenced file, by file name hash, sorted.
pub fn build_file_users(
    collection: &EffectDefinitionCollection,
    tileart: &[(u32, &[EffectDefinition])],
    name_of: &dyn Fn(&ParticleEffect) -> Option<String>,
    scripts: &[(u64, Vec<String>)],
) -> HashMap<u64, Vec<FileUser>> {
    let mut users: HashMap<u64, Vec<FileUser>> = HashMap::new();
    let mut add_particles = |definitions: &[EffectDefinition], user: FileUser| {
        for definition in definitions {
            definition.visit(&mut |definition| {
                if let EffectDefinition::Particle(particle) = definition
                    && let Some(name) = name_of(particle)
                {
                    users.entry(uocf::enhanced::effect_texture::effect_texture_hash(&name)).or_default().push(user);
                }
            });
        }
    };
    for (index, entry) in collection.entries.iter().enumerate() {
        add_particles(&entry.definitions, FileUser::Entry(index));
    }
    for (tile_id, definitions) in tileart {
        add_particles(definitions, FileUser::TileArt(*tile_id));
    }
    for (script, files) in scripts {
        for file in files {
            users.entry(uocf::enhanced::effect_texture::effect_texture_hash(file)).or_default().push(FileUser::Script(*script));
        }
    }
    for list in users.values_mut() {
        list.sort();
        list.dedup();
    }
    users
}

/// Opens an `EffectDefinitionCollection.uop`.
pub fn open_collection(path: &Path) -> Result<EffectDefinitionCollection, String> {
    EffectDefinitionCollection::load(path).map_err(|error| format!("{error:#}"))
}

/// Imports a KDL file; parse errors name the line and column.
pub fn import_kdl(path: &Path, strings: Option<&UoStringDictionary>) -> Result<EffectDefinitionCollection, String> {
    EffectDefinitionCollection::load_kdl(path, strings).map_err(|error| format!("{error:#}"))
}

pub fn export_kdl(
    collection: &EffectDefinitionCollection,
    path: &Path,
    strings: Option<&UoStringDictionary>,
) -> Result<(), String> {
    collection.write_kdl(path, strings).map_err(|error| format!("{error:#}"))
}

/// Writes a copy of the `source` package with exactly the entries of `collection`. Refuses to write over the
/// source or any of the `protected` files (the loaded client files).
pub fn save_collection(
    collection: &EffectDefinitionCollection,
    source: &Path,
    output: &Path,
    protected: &[&Path],
) -> Result<EffectCollectionChanges, String> {
    refuse_protected(output, std::iter::once(source).chain(protected.iter().copied()))?;
    collection.write_effect_definition_collection(source, output).map_err(|error| format!("{error:#}"))
}

/// Errors when `output` names one of the `protected` files.
pub fn refuse_protected<'a>(output: &Path, protected: impl IntoIterator<Item = &'a Path>) -> Result<(), String> {
    match protected.into_iter().find(|protected| same_file(protected, output)) {
        Some(protected) => Err(format!(
            "refusing to overwrite the loaded file {}: choose a new path",
            protected.display()
        )),
        None => Ok(()),
    }
}

/// True when both paths name the same existing file (or are equal as given).
pub fn same_file(left: &Path, right: &Path) -> bool {
    left == right
        || matches!(
            (std::fs::canonicalize(left), std::fs::canonicalize(right)),
            (Ok(left), Ok(right)) if left == right
        )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use uocf::enhanced::effect_definition::{
        encode_effect_definition_collection, effect_collection_entry_path, EffectIssueKind,
        EffectValidationContext, EFFECT_COLLECTION_ID_SCAN_END,
    };
    use uocf::enhanced::effect_definition::PARTICLE_LOD_DEFAULT;
    use uocf::uop_container::file::CompressionFlag;
    use uocf::uop_container::package::UopPackage;

    /// Overrides the directory the file tests write into (default: the system temp directory).
    pub(crate) const SCRATCH_DIR_ENV: &str = "UOCF_INSPECTOR_TEST_SCRATCH_DIR";
    /// Real Enhanced Client directory for the ignored tests.
    pub(crate) const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";
    const SOUND_A: u32 = 43;
    const SOUND_B: u32 = 44;
    const CLUMSY_ID: u32 = 5001;
    const CLUMSY_PACKET: u16 = 5002;

    pub(crate) fn scratch_dir(name: &str) -> PathBuf {
        let base = std::env::var_os(SCRATCH_DIR_ENV).map_or_else(std::env::temp_dir, PathBuf::from);
        let dir = base.join(format!("uocf_inspector_effects_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sound(sound_id: u32) -> EffectDefinition {
        EffectDefinition::Sound(SoundEffect { sound_id, value_08: NEW_SOUND_VALUE_08 })
    }

    fn sample() -> EffectDefinitionCollection {
        let random = EffectDefinition::Random(RandomEffect {
            entries: vec![
                RandomEffectEntry { weight: 60.0, children: vec![sound(SOUND_A)] },
                RandomEffectEntry { weight: 40.0, children: vec![sound(SOUND_B), sound(SOUND_A)] },
            ],
        });
        let lod = EffectDefinition::Lod(LodEffect {
            levels: vec![EffectMapEntry { key: 3, children: vec![sound(SOUND_A)] }],
        });
        EffectDefinitionCollection {
            entries: vec![
                EffectCollectionEntry { key: EffectEntryKey::Id(0), definitions: vec![sound(SOUND_A)] },
                EffectCollectionEntry { key: EffectEntryKey::Id(CLUMSY_ID), definitions: vec![lod, random] },
            ],
        }
    }

    fn document() -> EffectsDocument {
        EffectsDocument::new(sample(), CollectionSource::None)
    }

    fn field_key(field: &'static str) -> Option<EffectsEditKey> {
        Some(EffectsEditKey::Field { entry: 0, path: vec![PathStep::top(0)], field })
    }

    fn set_sound(collection: &mut EffectDefinitionCollection, sound_id: u32) {
        if let Some(EffectDefinition::Sound(effect)) = definition_at_mut(&mut collection.entries[0].definitions, &[PathStep::top(0)]) {
            effect.sound_id = sound_id;
        }
    }

    #[test]
    fn undo_and_redo_restore_snapshots_and_track_dirty() {
        let mut document = document();
        assert!(!document.is_dirty() && !document.can_undo());
        document.edit_collection(None, |collection| set_sound(collection, 1));
        assert!(document.collection_dirty() && !document.files_dirty());
        assert!(document.undo());
        assert_eq!(document.collection(), &sample());
        assert!(!document.is_dirty());
        assert!(document.redo());
        assert!(document.is_dirty());
        assert!(!document.redo());
    }

    #[test]
    fn edits_with_the_same_key_form_one_undo_step() {
        let mut document = document();
        for sound_id in 1..=3 {
            document.edit_collection(field_key("sound_id"), |collection| set_sound(collection, sound_id));
        }
        document.edit_collection(field_key("value_08"), |collection| set_sound(collection, 4));
        assert!(document.undo());
        assert!(document.undo());
        assert_eq!(document.collection(), &sample());
        assert!(!document.can_undo());

        document.edit_collection(field_key("sound_id"), |collection| set_sound(collection, 7));
        document.break_coalescing();
        document.edit_collection(field_key("sound_id"), |collection| set_sound(collection, 8));
        assert!(document.undo());
        assert!(document.can_undo(), "breaking the coalescing starts a new step");
    }

    #[test]
    fn edits_without_changes_are_not_recorded() {
        let mut document = document();
        let revision = document.revision();
        document.edit_collection(None, |_| ());
        assert!(!document.can_undo());
        assert_eq!(document.revision(), revision);
    }

    #[test]
    fn undo_history_is_bounded() {
        let mut document = document();
        for sound_id in 0..EFFECTS_UNDO_DEPTH as u32 + 10 {
            document.edit_collection(None, |collection| set_sound(collection, sound_id + 100));
        }
        let mut undone = 0;
        while document.undo() {
            undone += 1;
        }
        assert_eq!(undone, EFFECTS_UNDO_DEPTH);
    }

    #[test]
    fn file_changes_share_the_history_and_have_their_own_dirty_state() {
        let mut document = document();
        let payload: Arc<[u8]> = Arc::from(&b"particlesystem \"x\" 1 { }"[..]);
        let hash = uocf::enhanced::effect_texture::effect_texture_hash("x.ems");
        let key = Some(EffectsEditKey::FileText { hash });
        document.edit(key.clone(), |data| {
            data.files.insert(hash, FileOverride { name: "x.ems".into(), payload: Some(Arc::clone(&payload)) })
        });
        document.edit(key, |data| data.files.get_mut(&hash).unwrap().payload = None);
        assert!(document.files_dirty() && !document.collection_dirty());
        document.mark_files_saved();
        assert!(!document.is_dirty());
        assert!(document.undo(), "typing in one file is one undo step");
        assert!(document.files().is_empty());
        assert!(document.files_dirty());
    }

    #[test]
    fn replacing_the_collection_keeps_file_changes_and_restarts_history() {
        let mut document = document();
        document.edit(None, |data| {
            data.files.insert(1, FileOverride { name: "a.tga".into(), payload: None });
        });
        document.replace_collection(EffectDefinitionCollection::default(), CollectionSource::Kdl(PathBuf::from("x.kdl")));
        assert!(document.collection().entries.is_empty());
        assert_eq!(document.files().len(), 1);
        assert!(!document.can_undo());
        document.reset_files();
        assert!(document.files().is_empty() && !document.is_dirty());
    }

    #[test]
    fn packet_routes_invert_the_client_mapping() {
        use uocf::enhanced::effect_definition::{
            EFFECT_ID_PACKET_BIAS, PACKET_EFFECT_CLASSIC, PACKET_EFFECT_NONE, PACKET_EXPLODE_CLASSIC,
        };
        use PacketEffectPath::{FixedEffect, MovingEffect, MovingExplosion};
        let field = |path, value| PacketRoute::Field { path, value };
        assert_eq!(
            packet_routes_for_collection_id(CLUMSY_ID),
            [field(FixedEffect, CLUMSY_PACKET), field(MovingExplosion, CLUMSY_PACKET)]
        );
        assert_eq!(packet_routes_for_collection_id(0), [field(FixedEffect, 1), field(MovingExplosion, 1)]);
        let collection_9500 = u32::from(PACKET_EFFECT_NONE) - 1;
        let collection_9501 = u32::from(PACKET_EFFECT_NONE);
        assert_eq!(packet_routes_for_collection_id(collection_9500), [field(MovingExplosion, PACKET_EFFECT_NONE)]);
        assert_eq!(
            packet_routes_for_collection_id(collection_9501),
            [field(MovingExplosion, PACKET_EFFECT_CLASSIC)],
            "9501 only as explode effect 9502"
        );
        let lightning_value = u16::try_from(EFFECT_ID_LIGHTNING + EFFECT_ID_PACKET_BIAS).unwrap();
        assert_eq!(
            packet_routes_for_collection_id(EFFECT_ID_LIGHTNING),
            [field(FixedEffect, lightning_value), field(MovingExplosion, lightning_value), PacketRoute::Lightning]
        );
        assert!(packet_routes_for_collection_id(u32::from(u16::MAX)).is_empty());
        for path in PacketEffectPath::ALL {
            for value in 0..=u16::MAX {
                match describe_packet_effect(path, value) {
                    Ok(id) => assert!(packet_routes_for_collection_id(id).contains(&field(path, value)), "{path:?} {value}"),
                    Err(reason) => assert!(!reason.is_empty()),
                }
            }
        }
        assert_eq!(describe_packet_effect(FixedEffect, CLUMSY_PACKET), Ok(CLUMSY_ID));
        assert_eq!(describe_packet_effect(FixedEffect, PACKET_EFFECT_NONE), Err("plays nothing".to_string()));
        assert!(describe_packet_effect(FixedEffect, PACKET_EFFECT_CLASSIC).unwrap_err().contains("without its tileart"));
        assert!(describe_packet_effect(FixedEffect, 0).unwrap_err().contains("with its tileart"));
        assert!(describe_packet_effect(MovingEffect, CLUMSY_PACKET).unwrap_err().contains("plays no collection entry"));
        assert!(describe_packet_effect(MovingExplosion, PACKET_EXPLODE_CLASSIC).unwrap_err().contains("0x36CB"));
        assert_eq!(describe_packet_effect(MovingExplosion, PACKET_EFFECT_CLASSIC), Ok(collection_9501));
    }

    #[test]
    fn filters_by_id_packet_kind_and_text() {
        let collection = sample();
        let no_names = |_: &ParticleEffect| None;
        let visible = |filter: &DefinitionFilter| filter.visible_entries(&collection, &no_names);
        assert_eq!(visible(&DefinitionFilter::default()), [0, 1]);
        let by_text = |text: &str| DefinitionFilter { text: text.into(), ..Default::default() };
        assert_eq!(visible(&by_text("5001")), [1]);
        assert_eq!(visible(&by_text(&CLUMSY_PACKET.to_string())), [1], "the packet value matches");
        assert_eq!(visible(&by_text("0x1389")), [1]);
        assert_eq!(visible(&by_text("random")), [1], "kind labels match");
        assert_eq!(visible(&DefinitionFilter { kind: Some(DefinitionKind::Lod), ..Default::default() }), [1]);
        assert_eq!(visible(&DefinitionFilter { kind: Some(DefinitionKind::Sound), ..Default::default() }), [0, 1]);
        assert!(visible(&DefinitionFilter { particle: ParticleKindFilter::Chaos, ..Default::default() }).is_empty());

        let particle = EffectDefinition::Particle(match DefinitionKind::Particle.new_definition() {
            EffectDefinition::Particle(particle) => particle,
            _ => unreachable!(),
        });
        let named = |_: &ParticleEffect| Some("Fizzle.EMS".to_string());
        let filter = DefinitionFilter { text: "fizz".into(), particle: ParticleKindFilter::Chaos, ..Default::default() };
        assert!(filter.matches(&[], &[], &[particle.clone()], &named));
        let filter = DefinitionFilter { particle: ParticleKindFilter::Nif, ..Default::default() };
        assert!(!filter.matches(&[], &[], &[particle], &named));
    }

    #[test]
    fn entries_are_added_duplicated_renumbered_and_removed() {
        let mut collection = sample();
        assert_eq!(next_free_entry_id(&collection), CLUMSY_ID + 1);
        assert_eq!(add_entry(&mut collection, None), Ok(EffectsSelection::Entry { entry: 2 }));
        assert_eq!(collection.entries[2].key, EffectEntryKey::Id(CLUMSY_ID + 1));
        assert!(add_entry(&mut collection, Some(0)).is_err(), "ids are unique");
        assert_eq!(duplicate_entry(&mut collection, 1, 7), Ok(EffectsSelection::Entry { entry: 2 }));
        assert_eq!(collection.entries[2].definitions, collection.entries[1].definitions);
        assert!(set_entry_id(&mut collection, 2, CLUMSY_ID).is_err());
        assert!(set_entry_id(&mut collection, 2, 2).is_ok());
        assert!(set_entry_id(&mut collection, 2, 2).is_ok(), "an entry keeps its own id");
        assert_eq!(remove_entry(&mut collection, 3), Some(EffectsSelection::Entry { entry: 2 }));
        assert_eq!(collection.entries.len(), 3);
    }

    #[test]
    fn definitions_are_added_duplicated_moved_and_removed_at_any_depth() {
        let mut collection = sample();
        let random = [PathStep::top(1)];
        let list = ChildList::RandomEntry(1);
        let selection = add_definition(&mut collection, 1, &random, list, DefinitionKind::Wait).unwrap();
        let wait_path = vec![PathStep::top(1), PathStep { list, index: 2 }];
        assert_eq!(selection, EffectsSelection::Definition { entry: 1, path: wait_path.clone() });
        assert!(matches!(definition_at(&collection.entries[1].definitions, &wait_path), Some(EffectDefinition::Wait(_))));

        let selection = move_definition(&mut collection, 1, &wait_path, -5).unwrap();
        let moved = vec![PathStep::top(1), PathStep { list, index: 0 }];
        assert_eq!(selection, EffectsSelection::Definition { entry: 1, path: moved.clone() });
        let selection = duplicate_definition(&mut collection, 1, &moved).unwrap();
        assert_eq!(selection, EffectsSelection::Definition { entry: 1, path: vec![PathStep::top(1), PathStep { list, index: 1 }] });
        let EffectDefinition::Random(random_effect) = &collection.entries[1].definitions[1] else { panic!() };
        assert_eq!(random_effect.entries[1].children.len(), 4);

        let top = add_definition(&mut collection, 0, &[], ChildList::Top, DefinitionKind::Earthquake).unwrap();
        assert_eq!(top, EffectsSelection::Definition { entry: 0, path: vec![PathStep::top(1)] });
        assert_eq!(remove_definition(&mut collection, 0, &[PathStep::top(1)]), Some(EffectsSelection::Definition { entry: 0, path: vec![PathStep::top(0)] }));
        assert_eq!(remove_definition(&mut collection, 0, &[PathStep::top(0)]), Some(EffectsSelection::Entry { entry: 0 }));
        assert!(add_definition(&mut collection, 1, &random, ChildList::Children, DefinitionKind::Sound).is_none(), "a random has no plain children");
    }

    #[test]
    fn keyed_lists_follow_the_client_rules() {
        let mut lod = DefinitionKind::Lod.new_definition();
        assert_eq!(add_keyed_list(&mut lod), Some(ChildList::Level(0)));
        assert_eq!(add_keyed_list(&mut lod), Some(ChildList::Level(1)));
        let EffectDefinition::Lod(effect) = &lod else { panic!() };
        assert_eq!(effect.levels.iter().map(|level| level.key).collect::<Vec<_>>(), [FIRST_LOD_LEVEL, FIRST_LOD_LEVEL + 1]);
        let mut random = DefinitionKind::Random.new_definition();
        add_keyed_list(&mut random);
        add_keyed_list(&mut random);
        let EffectDefinition::Random(effect) = &random else { panic!() };
        assert_eq!(effect.entries[0].weight, RANDOM_WEIGHT_TOTAL);
        assert_eq!(effect.entries[1].weight, 0.0);
        assert!(remove_keyed_list(&mut random, ChildList::RandomEntry(0)));
        assert!(!remove_keyed_list(&mut random, ChildList::RandomEntry(5)));
        assert_eq!(add_keyed_list(&mut DefinitionKind::Sound.new_definition()), None);
    }

    #[test]
    fn selections_resolve_to_existing_items() {
        let collection = sample();
        let deep = EffectsSelection::Definition { entry: 1, path: vec![PathStep::top(1), PathStep { list: ChildList::RandomEntry(1), index: 9 }] };
        assert_eq!(deep.resolve(&collection), Some(EffectsSelection::Definition { entry: 1, path: vec![PathStep::top(1)] }));
        let gone = EffectsSelection::Definition { entry: 0, path: vec![PathStep::top(4)] };
        assert_eq!(gone.resolve(&collection), Some(EffectsSelection::Entry { entry: 0 }));
        assert_eq!(EffectsSelection::Entry { entry: 5 }.resolve(&collection), None);
    }

    #[test]
    fn issue_paths_select_their_definition() {
        let collection = sample();
        let definitions = &collection.entries[1].definitions;
        assert_eq!(
            parse_issue_path(definitions, "1/entry 1/0"),
            Some(vec![PathStep::top(1), PathStep { list: ChildList::RandomEntry(1), index: 0 }])
        );
        assert_eq!(
            parse_issue_path(definitions, "0/level 3/0"),
            Some(vec![PathStep::top(0), PathStep { list: ChildList::Level(0), index: 0 }])
        );
        assert_eq!(parse_issue_path(definitions, "0/level 9/0"), None);
        assert_eq!(parse_issue_path(definitions, "0/level 3"), Some(vec![PathStep::top(0)]));
        assert_eq!(parse_issue_path(definitions, "7"), None);

        let mut collection = collection;
        collection.entries[1].definitions[1] = EffectDefinition::Random(RandomEffect {
            entries: vec![RandomEffectEntry { weight: 10.0, children: vec![sound(SOUND_A)] }],
        });
        collection.entries[0].definitions.clear();
        let issues = collection.validate(&EffectValidationContext::default());
        let weight = issues.iter().find(|issue| matches!(issue.kind, EffectIssueKind::RandomWeightSum { .. })).unwrap();
        assert_eq!(issue_selection(&collection, weight), Some(EffectsSelection::Definition { entry: 1, path: vec![PathStep::top(1)] }));
        let empty = issues.iter().find(|issue| issue.kind == EffectIssueKind::EmptyEntry).unwrap();
        assert_eq!(issue_selection(&collection, empty), Some(EffectsSelection::Entry { entry: 0 }));
    }

    #[test]
    fn name_index_resolves_like_the_particle_reader() {
        let strings = ["first.nif", "Second.EMS"];
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        for value in strings {
            payload.extend_from_slice(&(value.len() as u16).to_le_bytes());
            payload.extend_from_slice(value.as_bytes());
        }
        let dictionary = UoStringDictionary::from_payload_bytes(&payload, "test dictionary").unwrap();
        let index = NameIndex::new(&dictionary);
        let name_index = index.name_index("second.ems").unwrap();
        let EffectDefinition::Particle(mut particle) = DefinitionKind::Particle.new_definition() else { panic!() };
        particle.name_index = name_index;
        assert_eq!(particle_name(&particle, Some(&dictionary)).as_deref(), Some("Second.EMS"));
        assert!(index.name_index("missing.nif").is_none());
        assert!(is_chaos_name("Second.EMS") && !is_chaos_name("first.nif"));
    }

    #[test]
    fn file_users_link_files_to_entries_tileart_and_scripts() {
        let mut collection = sample();
        let EffectDefinition::Particle(particle) = DefinitionKind::Particle.new_definition() else { panic!() };
        collection.entries[0].definitions.push(EffectDefinition::Particle(particle.clone()));
        let tileart_definitions = vec![EffectDefinition::Particle(particle)];
        let name_of = |_: &ParticleEffect| Some("fire.ems".to_string());
        let script = uocf::enhanced::effect_texture::effect_texture_hash("fire.ems");
        let users = build_file_users(
            &collection,
            &[(0x0FAC, tileart_definitions.as_slice())],
            &name_of,
            &[(script, vec!["Flame.TGA".to_string()])],
        );
        assert_eq!(users[&script], [FileUser::Entry(0), FileUser::TileArt(0x0FAC)]);
        assert_eq!(users[&uocf::enhanced::effect_texture::effect_texture_hash("flame.tga")], [FileUser::Script(script)]);
    }

    #[test]
    fn saves_and_reopens_the_collection() {
        let dir = scratch_dir("collection");
        let source = dir.join("EffectDefinitionCollection.uop");
        let mut package = UopPackage::new_default();
        let payload = encode_effect_definition_collection(&[sound(SOUND_A)]);
        package.add_file_from_memory(&payload, &effect_collection_entry_path(0), CompressionFlag::Zlib).unwrap();
        package.finalize_and_save(&source).unwrap();

        let collection = sample();
        let output = dir.join("edited.uop");
        let changes = save_collection(&collection, &source, &output, &[]).unwrap();
        assert_eq!(changes.added, 1);
        assert_eq!(open_collection(&output).unwrap(), collection);
        let error = save_collection(&collection, &source, &dir.join(".").join("EffectDefinitionCollection.uop"), &[]).unwrap_err();
        assert!(error.contains("refusing to overwrite"), "{error}");
        let protected = dir.join("protected.uop");
        std::fs::copy(&source, &protected).unwrap();
        assert!(save_collection(&collection, &source, &protected, &[&protected]).is_err());

        let kdl = dir.join("effects.kdl");
        export_kdl(&collection, &kdl, None).unwrap();
        assert_eq!(import_kdl(&kdl, None).unwrap(), collection);
        std::fs::write(&kdl, "entry 1 {\n    sound \"x\"\n}\n").unwrap();
        let error = import_kdl(&kdl, None).unwrap_err();
        assert!(error.contains("line 2"), "KDL errors name the line: {error}");
        assert!(EFFECT_COLLECTION_ID_SCAN_END > CLUMSY_ID);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn lod_default_is_a_client_level() {
        assert!(PARTICLE_LOD_DEFAULT >= FIRST_LOD_LEVEL);
    }
}
