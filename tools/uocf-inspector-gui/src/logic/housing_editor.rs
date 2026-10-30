//! Editing model of the Housing panel: the edited `housing.bin` with its undo history, the selection, the
//! piece filters, the edit operations and the file operations. No egui here; `ui/housing.rs` draws it.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use uocf::enhanced::housing::{
    HousingIssue, HousingPiece, HousingPieceKey, HousingPieces, HousingSlot, HousingSlotList, HousingTileType,
    HOUSING_CLIENT_TILE_TYPES, HOUSING_FEATURE_NO_EXPANSION,
};

/// Undo snapshots kept per document; older ones are dropped.
pub const HOUSING_UNDO_DEPTH: usize = 100;
/// Slot given to the first entry of an empty slot list (the client lists start at slot 1).
pub const HOUSING_FIRST_SLOT: u32 = 1;
/// Feature mask of a new piece in a new category: always granted.
pub const HOUSING_NEW_PIECE_FEATURE_MASK: u32 = HOUSING_FEATURE_NO_EXPANSION;

/// Where the edited pieces were opened from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HousingDocumentSource {
    /// `housing.bin` of the loaded client's `MultiCollection.uop`.
    ClientPackage(PathBuf),
    /// A loose `housing.bin` file.
    HousingBin(PathBuf),
    /// A KDL text file.
    Kdl(PathBuf),
}

impl HousingDocumentSource {
    pub fn label(&self) -> String {
        match self {
            Self::ClientPackage(path) => format!("client {}", path.display()),
            Self::HousingBin(path) => format!("housing.bin {}", path.display()),
            Self::Kdl(path) => format!("KDL {}", path.display()),
        }
    }
}

/// A field of the edited data, for undo coalescing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HousingField {
    TileType,
    Category,
    Group,
    FeatureMask,
    Tooltip,
    Slot { list: HousingSlotList, row: usize },
    SlotId { list: HousingSlotList, row: usize },
}

/// Identifies a field edit: consecutive edits with the same key (typing, dragging a value) form one undo step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HousingEditKey {
    pub entry: usize,
    pub piece: Option<usize>,
    pub field: HousingField,
}

/// Bounded snapshot stack of [`HousingPieces`].
#[derive(Debug, Default)]
pub struct HousingUndoHistory {
    undo: VecDeque<HousingPieces>,
    redo: Vec<HousingPieces>,
    last_key: Option<HousingEditKey>,
}

impl HousingUndoHistory {
    /// Records the state before an edit. An edit with the same key as the previous one joins its undo step.
    pub fn record(&mut self, before: HousingPieces, key: Option<HousingEditKey>) {
        self.redo.clear();
        if key.is_some() && key == self.last_key {
            return;
        }
        self.last_key = key;
        push_bounded(&mut self.undo, before);
    }

    /// Ends the current coalesced edit: the next edit is a new undo step.
    pub fn break_coalescing(&mut self) {
        self.last_key = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Replaces `current` with the previous snapshot; false when there is none.
    pub fn undo(&mut self, current: &mut HousingPieces) -> bool {
        let Some(previous) = self.undo.pop_back() else {
            return false;
        };
        self.redo.push(std::mem::replace(current, previous));
        self.last_key = None;
        true
    }

    /// Replaces `current` with the next snapshot; false when there is none.
    pub fn redo(&mut self, current: &mut HousingPieces) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        push_bounded(&mut self.undo, std::mem::replace(current, next));
        self.last_key = None;
        true
    }
}

fn push_bounded(stack: &mut VecDeque<HousingPieces>, snapshot: HousingPieces) {
    stack.push_back(snapshot);
    while stack.len() > HOUSING_UNDO_DEPTH {
        stack.pop_front();
    }
}

/// The edited pieces with their opened and saved states and the undo history.
#[derive(Debug)]
pub struct HousingDocument {
    pieces: HousingPieces,
    opened: HousingPieces,
    saved: HousingPieces,
    source: HousingDocumentSource,
    history: HousingUndoHistory,
    dirty: bool,
    /// Increased on every change of `pieces`, for the caches of the panel.
    revision: u64,
}

impl HousingDocument {
    pub fn new(pieces: HousingPieces, source: HousingDocumentSource) -> Self {
        Self {
            opened: pieces.clone(),
            saved: pieces.clone(),
            pieces,
            source,
            history: HousingUndoHistory::default(),
            dirty: false,
            revision: 0,
        }
    }

    pub fn pieces(&self) -> &HousingPieces {
        &self.pieces
    }

    pub fn source(&self) -> &HousingDocumentSource {
        &self.source
    }

    /// True when the pieces differ from the last opened or saved state.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// True when the pieces differ from the opened state ("revert to loaded" has something to do).
    pub fn differs_from_opened(&self) -> bool {
        self.pieces != self.opened
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Applies an edit; when it changes the pieces, the previous state goes on the undo stack.
    pub fn edit<R>(&mut self, key: Option<HousingEditKey>, edit: impl FnOnce(&mut HousingPieces) -> R) -> R {
        let before = self.pieces.clone();
        let result = edit(&mut self.pieces);
        if self.pieces != before {
            self.history.record(before, key);
            self.changed();
        }
        result
    }

    pub fn break_coalescing(&mut self) {
        self.history.break_coalescing();
    }

    pub fn undo(&mut self) -> bool {
        let undone = self.history.undo(&mut self.pieces);
        if undone {
            self.changed();
        }
        undone
    }

    pub fn redo(&mut self) -> bool {
        let redone = self.history.redo(&mut self.pieces);
        if redone {
            self.changed();
        }
        redone
    }

    /// Goes back to the opened pieces, as one undoable edit.
    pub fn revert_to_opened(&mut self) {
        let opened = self.opened.clone();
        self.edit(None, |pieces| *pieces = opened);
    }

    /// Records the current pieces as saved.
    pub fn mark_saved(&mut self) {
        self.saved = self.pieces.clone();
        self.dirty = false;
    }

    fn changed(&mut self) {
        self.revision += 1;
        self.dirty = self.pieces != self.saved;
    }
}

/// What the panel shows on the right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HousingSelection {
    TileType { entry: usize },
    Category { entry: usize, category: u32 },
    Piece { entry: usize, piece: usize },
}

impl HousingSelection {
    pub fn entry(self) -> usize {
        match self {
            Self::TileType { entry } | Self::Category { entry, .. } | Self::Piece { entry, .. } => entry,
        }
    }

    /// The selection if it still exists in `pieces`, else the closest existing parent.
    pub fn resolve(self, pieces: &HousingPieces) -> Option<Self> {
        let tile_type = pieces.tile_types.get(self.entry())?;
        let resolved = match self {
            Self::TileType { .. } => self,
            Self::Category { entry, category } => {
                if tile_type.pieces.iter().any(|piece| piece.category == category) {
                    self
                } else {
                    Self::TileType { entry }
                }
            }
            Self::Piece { entry, piece } => {
                if piece < tile_type.pieces.len() {
                    self
                } else {
                    Self::TileType { entry }
                }
            }
        };
        Some(resolved)
    }
}

/// Index of the tile type entry the client keeps for `tile_type` (the last one).
pub fn kept_entry(pieces: &HousingPieces, tile_type: u32) -> Option<usize> {
    pieces.tile_types.iter().rposition(|entry| entry.tile_type == tile_type)
}

/// Visible piece indices of a tile type entry grouped by category (window rows in ascending order), each in
/// file order.
pub fn categories_of(entry: &HousingTileType, visible: &[usize]) -> BTreeMap<u32, Vec<usize>> {
    let mut categories = BTreeMap::<u32, Vec<usize>>::new();
    for &index in visible {
        categories.entry(entry.pieces[index].category).or_default().push(index);
    }
    categories
}

/// Piece filters of the tree. Empty fields do not filter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HousingPieceFilter {
    /// Keep pieces whose feature mask has this bit.
    pub feature_bit: Option<u32>,
    /// Keep pieces using this tile id in a solid or window slot ([`HousingPieces::tile_index`]).
    pub tile_id: Option<u32>,
    /// Keep pieces whose tooltip cliloc number, or the text of their category tooltip, contains this text
    /// (case insensitive).
    pub text: String,
}

impl HousingPieceFilter {
    pub fn is_active(&self) -> bool {
        self.feature_bit.is_some() || self.tile_id.is_some() || !self.text.trim().is_empty()
    }

    /// The pieces that pass, as piece indices per tile type entry.
    pub fn visible_pieces(
        &self,
        pieces: &HousingPieces,
        cliloc_text: &dyn Fn(u32) -> Option<String>,
    ) -> Vec<Vec<usize>> {
        let tile_keys: Option<BTreeSet<HousingPieceKey>> = self.tile_id.map(|tile_id| {
            pieces
                .tile_index()
                .get(&tile_id)
                .map(|slots| slots.iter().map(|slot| slot.piece).collect())
                .unwrap_or_default()
        });
        let query = self.text.trim().to_lowercase();
        pieces
            .tile_types
            .iter()
            .map(|entry| {
                (0..entry.pieces.len())
                    .filter(|&index| {
                        let piece = &entry.pieces[index];
                        self.feature_bit.is_none_or(|bit| piece.feature_mask & bit != 0)
                            && tile_keys.as_ref().is_none_or(|keys| keys.contains(&piece.key(entry.tile_type)))
                            && (query.is_empty() || text_matches(entry, piece, &query, cliloc_text))
                    })
                    .collect()
            })
            .collect()
    }
}

fn text_matches(
    entry: &HousingTileType,
    piece: &HousingPiece,
    query: &str,
    cliloc_text: &dyn Fn(u32) -> Option<String>,
) -> bool {
    let category_tooltip = entry_category_tooltip(entry, piece.category);
    [Some(piece.tooltip_cliloc).filter(|&cliloc| cliloc != 0), category_tooltip]
        .into_iter()
        .flatten()
        .any(|cliloc| {
            cliloc.to_string().contains(query)
                || cliloc_text(cliloc).is_some_and(|text| text.to_lowercase().contains(query))
        })
}

/// The category name cliloc within one tile type entry: the first nonzero tooltip of its pieces.
pub fn entry_category_tooltip(entry: &HousingTileType, category: u32) -> Option<u32> {
    entry
        .pieces
        .iter()
        .filter(|piece| piece.category == category)
        .map(|piece| piece.tooltip_cliloc)
        .find(|&cliloc| cliloc != 0)
}

/// Adds a tile type entry: the first client tile type without an entry, or the one after the highest.
pub fn add_tile_type(pieces: &mut HousingPieces) -> HousingSelection {
    let present: BTreeSet<u32> = pieces.tile_types.iter().map(|entry| entry.tile_type).collect();
    let tile_type = HOUSING_CLIENT_TILE_TYPES
        .clone()
        .find(|tile_type| !present.contains(tile_type))
        .unwrap_or_else(|| present.last().map_or(*HOUSING_CLIENT_TILE_TYPES.start(), |last| last.saturating_add(1)));
    pieces.tile_types.push(HousingTileType { tile_type, pieces: Vec::new() });
    HousingSelection::TileType { entry: pieces.tile_types.len() - 1 }
}

/// Removes a tile type entry; selects the previous entry, if any.
pub fn remove_tile_type(pieces: &mut HousingPieces, entry: usize) -> Option<HousingSelection> {
    if entry >= pieces.tile_types.len() {
        return None;
    }
    pieces.tile_types.remove(entry);
    let remaining = pieces.tile_types.len();
    (remaining > 0).then(|| HousingSelection::TileType { entry: entry.min(remaining - 1) })
}

/// Adds a piece to a tile type entry. With a category, the piece is a new group of it (after its highest
/// group, with the category's feature mask) placed after the category's last piece; without one, it is the
/// first piece of a new category after the highest. Tooltip and lists start empty.
pub fn add_piece(pieces: &mut HousingPieces, entry: usize, category: Option<u32>) -> Option<HousingSelection> {
    let tile_type = pieces.tile_types.get_mut(entry)?;
    let mut piece = HousingPiece {
        category: 0,
        group: 0,
        feature_mask: HOUSING_NEW_PIECE_FEATURE_MASK,
        tooltip_cliloc: 0,
        solid: Vec::new(),
        multis: Vec::new(),
        windows: Vec::new(),
    };
    let last_of_category =
        category.and_then(|category| tile_type.pieces.iter().rposition(|piece| piece.category == category));
    let index = match (category, last_of_category) {
        (Some(category), Some(last)) => {
            let category_pieces = tile_type.pieces.iter().filter(|piece| piece.category == category);
            piece.category = category;
            piece.group = category_pieces.map(|piece| piece.group).max().map_or(0, |group| group.saturating_add(1));
            piece.feature_mask = tile_type.pieces[last].feature_mask;
            last + 1
        }
        _ => {
            piece.category = tile_type
                .pieces
                .iter()
                .map(|piece| piece.category)
                .max()
                .map_or(0, |category| category.saturating_add(1));
            tile_type.pieces.len()
        }
    };
    tile_type.pieces.insert(index, piece);
    Some(HousingSelection::Piece { entry, piece: index })
}

/// Inserts a copy of a piece right after it.
pub fn duplicate_piece(pieces: &mut HousingPieces, entry: usize, piece: usize) -> Option<HousingSelection> {
    let tile_type = pieces.tile_types.get_mut(entry)?;
    let copy = tile_type.pieces.get(piece)?.clone();
    tile_type.pieces.insert(piece + 1, copy);
    Some(HousingSelection::Piece { entry, piece: piece + 1 })
}

/// Deletes a piece; selects the piece now at its index, the previous one, or the tile type.
pub fn delete_piece(pieces: &mut HousingPieces, entry: usize, piece: usize) -> Option<HousingSelection> {
    let tile_type = pieces.tile_types.get_mut(entry)?;
    if piece >= tile_type.pieces.len() {
        return None;
    }
    tile_type.pieces.remove(piece);
    let remaining = tile_type.pieces.len();
    Some(if remaining == 0 {
        HousingSelection::TileType { entry }
    } else {
        HousingSelection::Piece { entry, piece: piece.min(remaining - 1) }
    })
}

/// Moves a piece by `delta` positions within its tile type entry (clamped); returns its new selection.
pub fn move_piece(pieces: &mut HousingPieces, entry: usize, piece: usize, delta: isize) -> Option<HousingSelection> {
    let tile_type = pieces.tile_types.get_mut(entry)?;
    if piece >= tile_type.pieces.len() {
        return None;
    }
    let target = piece.saturating_add_signed(delta).min(tile_type.pieces.len() - 1);
    let moved = tile_type.pieces.remove(piece);
    tile_type.pieces.insert(target, moved);
    Some(HousingSelection::Piece { entry, piece: target })
}

/// Appends a slot to a list: the slot after the highest (or [`HOUSING_FIRST_SLOT`]), id 0. Returns its row.
pub fn add_slot(piece: &mut HousingPiece, list: HousingSlotList) -> usize {
    let slots = piece.slots_mut(list);
    let slot = slots.iter().map(|slot| slot.slot).max().map_or(HOUSING_FIRST_SLOT, |slot| slot.saturating_add(1));
    slots.push(HousingSlot { slot, id: 0 });
    slots.len() - 1
}

/// Removes a row of a slot list; false when there is no such row.
pub fn remove_slot(piece: &mut HousingPiece, list: HousingSlotList, row: usize) -> bool {
    let slots = piece.slots_mut(list);
    if row >= slots.len() {
        return false;
    }
    slots.remove(row);
    true
}

/// What a validation issue is about, to select it in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HousingIssueTarget {
    TileType(u32),
    Category { tile_type: u32, category: u32 },
    Piece { key: HousingPieceKey, slot: Option<(HousingSlotList, u32)> },
}

pub fn issue_target(issue: &HousingIssue) -> HousingIssueTarget {
    use HousingIssue as Issue;
    match *issue {
        Issue::DuplicateTileType { tile_type } | Issue::UnknownTileType { tile_type } => {
            HousingIssueTarget::TileType(tile_type)
        }
        Issue::DuplicatePiece { piece }
        | Issue::UnexpectedList { piece, .. }
        | Issue::MissingTooltipCliloc { piece, .. }
        | Issue::MaskOutsideExpansionFilter { piece, .. } => HousingIssueTarget::Piece { key: piece, slot: None },
        Issue::DuplicateSlot { slot, .. } | Issue::MissingTile { slot, .. } | Issue::MissingMulti { slot, .. } => {
            HousingIssueTarget::Piece { key: slot.piece, slot: Some((slot.list, slot.slot)) }
        }
        Issue::MixedFeatureMask { tile_type, category, .. }
        | Issue::ConflictingTooltips { tile_type, category, .. }
        | Issue::CategoryWithoutTooltip { tile_type, category }
        | Issue::CategoryGap { tile_type, category } => HousingIssueTarget::Category { tile_type, category },
    }
}

/// The selection for an issue target: in the tile type entry the client keeps, else in any entry of the tile
/// type. A category without pieces (a gap) selects its tile type.
pub fn find_target(pieces: &HousingPieces, target: HousingIssueTarget) -> Option<HousingSelection> {
    let tile_type = match target {
        HousingIssueTarget::TileType(tile_type) => tile_type,
        HousingIssueTarget::Category { tile_type, .. } => tile_type,
        HousingIssueTarget::Piece { key, .. } => key.tile_type,
    };
    let kept = kept_entry(pieces, tile_type)?;
    let entries = std::iter::once(kept).chain(
        (0..pieces.tile_types.len()).filter(move |&entry| entry != kept && pieces.tile_types[entry].tile_type == tile_type),
    );
    match target {
        HousingIssueTarget::TileType(_) => Some(HousingSelection::TileType { entry: kept }),
        HousingIssueTarget::Category { category, .. } => Some(
            entries
                .clone()
                .find(|&entry| pieces.tile_types[entry].pieces.iter().any(|piece| piece.category == category))
                .map_or(HousingSelection::TileType { entry: kept }, |entry| HousingSelection::Category { entry, category }),
        ),
        HousingIssueTarget::Piece { key, .. } => entries
            .clone()
            .find_map(|entry| {
                let pieces = &pieces.tile_types[entry].pieces;
                let piece = pieces.iter().position(|piece| piece.category == key.category && piece.group == key.group)?;
                Some(HousingSelection::Piece { entry, piece })
            })
            .or(Some(HousingSelection::TileType { entry: kept })),
    }
}

/// Parses a tile or multi id typed as decimal or `0x` hex; `None` for empty or invalid text.
pub fn parse_id(text: &str) -> Option<u32> {
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => text.parse().ok(),
    }
}

/// Opens `housing.bin` from a `MultiCollection.uop`.
pub fn open_client_package(path: &Path) -> Result<HousingDocument, String> {
    match HousingPieces::load(path) {
        Ok(Some(pieces)) => Ok(HousingDocument::new(pieces, HousingDocumentSource::ClientPackage(path.to_path_buf()))),
        Ok(None) => Err(format!("{} has no housing.bin", path.display())),
        Err(error) => Err(format!("{error:#}")),
    }
}

/// Opens a loose `housing.bin`.
pub fn open_housing_bin(path: &Path) -> Result<HousingDocument, String> {
    HousingPieces::load_housing_bin(path)
        .map(|pieces| HousingDocument::new(pieces, HousingDocumentSource::HousingBin(path.to_path_buf())))
        .map_err(|error| format!("{error:#}"))
}

/// Imports a KDL file; parse errors name the line and column.
pub fn import_kdl(path: &Path) -> Result<HousingDocument, String> {
    HousingPieces::load_kdl(path)
        .map(|pieces| HousingDocument::new(pieces, HousingDocumentSource::Kdl(path.to_path_buf())))
        .map_err(|error| format!("{error:#}"))
}

/// Writes a copy of the client's `MultiCollection.uop` (`source`) with only `housing.bin` replaced. Refuses
/// to write over `source`.
pub fn save_multi_collection(pieces: &HousingPieces, source: &Path, output: &Path) -> Result<(), String> {
    if same_file(source, output) {
        return Err(format!(
            "refusing to overwrite the loaded client package {}: choose a new path",
            source.display()
        ));
    }
    pieces.write_multi_collection(source, output).map_err(|error| format!("{error:#}"))
}

pub fn save_housing_bin(pieces: &HousingPieces, path: &Path) -> Result<(), String> {
    pieces.write_housing_bin(path).map_err(|error| format!("{error:#}"))
}

pub fn export_kdl(pieces: &HousingPieces, path: &Path) -> Result<(), String> {
    pieces.write_kdl(path).map_err(|error| format!("{error:#}"))
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
mod tests {
    use super::*;
    use uocf::enhanced::housing::{
        HousingIssueSeverity, HousingSlotRef, HousingValidationContext, HOUSING_FEATURE_AOW, HOUSING_FEATURE_SA,
        HOUSING_TILE_TYPE_FLOOR, HOUSING_TILE_TYPE_STAIRS, HOUSING_TILE_TYPE_WALL,
    };
    use uocf::enhanced::multis::{multi_collection_path, HOUSING_PATH};
    use uocf::uop_container::file::CompressionFlag;
    use uocf::uop_container::package::UopPackage;

    const STAIRS_TOOLTIP: u32 = 1021826;
    const WALL_TOOLTIP: u32 = 1156116;
    const STAIRS_TILE: u32 = 0x071F;
    const WALL_TILE: u32 = 0x0009;
    const WINDOW_TILE: u32 = 0x0010;
    const STAIRS_MULTI: u32 = 7600;
    const WALL_CATEGORY: u32 = 4;
    /// Overrides the directory the file tests write into (default: the system temp directory).
    const SCRATCH_DIR_ENV: &str = "UOCF_INSPECTOR_TEST_SCRATCH_DIR";
    /// Real Enhanced Client directory for the ignored test.
    const EC_CLIENT_DIR_ENV: &str = "UO_EC_CLIENT_DIR";

    fn piece(category: u32, group: u32, feature_mask: u32, tooltip_cliloc: u32) -> HousingPiece {
        HousingPiece {
            category,
            group,
            feature_mask,
            tooltip_cliloc,
            solid: Vec::new(),
            multis: Vec::new(),
            windows: Vec::new(),
        }
    }

    fn sample() -> HousingPieces {
        let mut stairs = piece(0, 0, HOUSING_FEATURE_NO_EXPANSION, STAIRS_TOOLTIP);
        stairs.solid.push(HousingSlot { slot: 1, id: STAIRS_TILE });
        stairs.multis.push(HousingSlot { slot: 1, id: STAIRS_MULTI });
        let mut wall = piece(WALL_CATEGORY, 0, HOUSING_FEATURE_NO_EXPANSION, WALL_TOOLTIP);
        wall.solid.push(HousingSlot { slot: 1, id: WALL_TILE });
        let mut wall_window = piece(WALL_CATEGORY, 1, HOUSING_FEATURE_AOW, 0);
        wall_window.windows.push(HousingSlot { slot: 1, id: WINDOW_TILE });
        HousingPieces {
            tile_types: vec![
                HousingTileType { tile_type: HOUSING_TILE_TYPE_STAIRS, pieces: vec![stairs] },
                HousingTileType { tile_type: HOUSING_TILE_TYPE_WALL, pieces: vec![wall, wall_window] },
            ],
        }
    }

    fn key(entry: usize, piece: usize, field: HousingField) -> Option<HousingEditKey> {
        Some(HousingEditKey { entry, piece: Some(piece), field })
    }

    fn document() -> HousingDocument {
        HousingDocument::new(sample(), HousingDocumentSource::Kdl(PathBuf::from("sample.kdl")))
    }

    #[test]
    fn undo_and_redo_restore_snapshots_and_track_dirty() {
        let mut document = document();
        assert!(!document.is_dirty() && !document.can_undo());

        document.edit(key(1, 0, HousingField::Group), |pieces| pieces.tile_types[1].pieces[0].group = 2);
        assert!(document.is_dirty() && document.can_undo() && !document.can_redo());
        let revision = document.revision();

        assert!(document.undo());
        assert_eq!(document.pieces(), &sample());
        assert!(!document.is_dirty() && document.can_redo());
        assert!(document.revision() > revision);

        assert!(document.redo());
        assert_eq!(document.pieces().tile_types[1].pieces[0].group, 2);
        assert!(!document.redo(), "nothing left to redo");
    }

    #[test]
    fn edits_with_the_same_key_form_one_undo_step() {
        let mut document = document();
        let tooltip = key(0, 0, HousingField::Tooltip);
        for value in [1, 12, 123] {
            document.edit(tooltip, |pieces| pieces.tile_types[0].pieces[0].tooltip_cliloc = value);
        }
        document.edit(key(0, 0, HousingField::Category), |pieces| pieces.tile_types[0].pieces[0].category = 1);

        assert!(document.undo());
        assert_eq!(document.pieces().tile_types[0].pieces[0].tooltip_cliloc, 123);
        assert!(document.undo());
        assert_eq!(document.pieces(), &sample(), "the three tooltip edits are one step");
        assert!(!document.undo());

        // A break starts a new step even with the same key.
        document.edit(tooltip, |pieces| pieces.tile_types[0].pieces[0].tooltip_cliloc = 1);
        document.break_coalescing();
        document.edit(tooltip, |pieces| pieces.tile_types[0].pieces[0].tooltip_cliloc = 2);
        assert!(document.undo());
        assert_eq!(document.pieces().tile_types[0].pieces[0].tooltip_cliloc, 1);
    }

    #[test]
    fn edits_without_changes_are_not_recorded() {
        let mut document = document();

        document.edit(None, |pieces| pieces.tile_types[0].pieces[0].group = 0);

        assert!(!document.can_undo());
        assert_eq!(document.revision(), 0);
    }

    #[test]
    fn undo_history_is_bounded() {
        let mut history = HousingUndoHistory::default();
        let mut current = HousingPieces::default();
        let extra_snapshots = 5;
        for _ in 0..HOUSING_UNDO_DEPTH + extra_snapshots {
            history.record(HousingPieces::default(), None);
        }

        let mut undone = 0;
        while history.undo(&mut current) {
            undone += 1;
        }
        assert_eq!(undone, HOUSING_UNDO_DEPTH);
    }

    #[test]
    fn revert_and_save_move_the_baselines() {
        let mut document = document();
        document.edit(None, |pieces| pieces.tile_types.pop());
        document.mark_saved();
        assert!(!document.is_dirty());
        assert!(document.differs_from_opened());

        document.revert_to_opened();

        assert_eq!(document.pieces(), &sample());
        assert!(document.is_dirty(), "the reverted pieces differ from the saved ones");
        assert!(document.undo(), "revert is undoable");
        assert_eq!(document.pieces().tile_types.len(), 1);
    }

    #[test]
    fn filters_by_feature_tile_and_text() {
        let pieces = sample();
        let texts = |cliloc| (cliloc == WALL_TOOLTIP).then(|| "Shadowguard Walls".to_string());

        let by_feature = HousingPieceFilter { feature_bit: Some(HOUSING_FEATURE_AOW), ..Default::default() };
        assert_eq!(by_feature.visible_pieces(&pieces, &texts), [vec![], vec![1]]);

        let by_tile = HousingPieceFilter { tile_id: Some(WINDOW_TILE), ..Default::default() };
        assert_eq!(by_tile.visible_pieces(&pieces, &texts), [vec![], vec![1]]);
        let unused_tile = HousingPieceFilter { tile_id: Some(STAIRS_MULTI), ..Default::default() };
        assert_eq!(unused_tile.visible_pieces(&pieces, &texts), [Vec::<usize>::new(), vec![]], "multi ids are not tiles");

        // The window group has no tooltip of its own; its category's tooltip text matches.
        let by_text = HousingPieceFilter { text: " shadowGUARD ".to_string(), ..Default::default() };
        assert_eq!(by_text.visible_pieces(&pieces, &texts), [vec![], vec![0, 1]]);
        let by_number = HousingPieceFilter { text: STAIRS_TOOLTIP.to_string(), ..Default::default() };
        assert_eq!(by_number.visible_pieces(&pieces, &texts), [vec![0], vec![]]);

        let combined = HousingPieceFilter { feature_bit: Some(HOUSING_FEATURE_SA), ..by_text };
        assert!(combined.is_active());
        assert_eq!(combined.visible_pieces(&pieces, &texts), [Vec::<usize>::new(), vec![]]);
        assert!(!HousingPieceFilter::default().is_active());
        assert_eq!(HousingPieceFilter::default().visible_pieces(&pieces, &texts), [vec![0], vec![0, 1]]);
    }

    #[test]
    fn categories_group_visible_pieces_in_row_order() {
        let mut entry = sample().tile_types[1].clone();
        entry.pieces.insert(0, piece(WALL_CATEGORY + 1, 0, HOUSING_FEATURE_NO_EXPANSION, 0));

        let categories = categories_of(&entry, &[0, 1, 2]);

        assert_eq!(categories, BTreeMap::from([(WALL_CATEGORY, vec![1, 2]), (WALL_CATEGORY + 1, vec![0])]));
    }

    #[test]
    fn add_piece_adds_a_group_or_a_category() {
        let mut pieces = sample();

        let group = add_piece(&mut pieces, 1, Some(WALL_CATEGORY)).unwrap();
        assert_eq!(group, HousingSelection::Piece { entry: 1, piece: 2 });
        let added = &pieces.tile_types[1].pieces[2];
        assert_eq!((added.category, added.group, added.feature_mask), (WALL_CATEGORY, 2, HOUSING_FEATURE_AOW));

        let category = add_piece(&mut pieces, 1, None).unwrap();
        assert_eq!(category, HousingSelection::Piece { entry: 1, piece: 3 });
        let added = &pieces.tile_types[1].pieces[3];
        assert_eq!((added.category, added.group, added.feature_mask), (WALL_CATEGORY + 1, 0, HOUSING_NEW_PIECE_FEATURE_MASK));

        let mut empty = HousingPieces::default();
        add_tile_type(&mut empty);
        add_piece(&mut empty, 0, None);
        assert_eq!(empty.tile_types[0].pieces[0].category, 0);
        assert_eq!(add_piece(&mut empty, 1, None), None);
    }

    #[test]
    fn duplicate_delete_and_move_pieces() {
        let mut pieces = sample();

        assert_eq!(duplicate_piece(&mut pieces, 1, 0), Some(HousingSelection::Piece { entry: 1, piece: 1 }));
        assert_eq!(pieces.tile_types[1].pieces[1], pieces.tile_types[1].pieces[0]);

        assert_eq!(move_piece(&mut pieces, 1, 0, 1), Some(HousingSelection::Piece { entry: 1, piece: 1 }));
        assert_eq!(move_piece(&mut pieces, 1, 1, isize::MAX), Some(HousingSelection::Piece { entry: 1, piece: 2 }));
        assert_eq!(pieces.tile_types[1].pieces[2].group, 0);
        assert_eq!(move_piece(&mut pieces, 1, 2, -isize::MAX), Some(HousingSelection::Piece { entry: 1, piece: 0 }));

        assert_eq!(delete_piece(&mut pieces, 1, 2), Some(HousingSelection::Piece { entry: 1, piece: 1 }));
        assert_eq!(delete_piece(&mut pieces, 0, 0), Some(HousingSelection::TileType { entry: 0 }));
        assert_eq!(delete_piece(&mut pieces, 0, 0), None);
        assert_eq!(pieces.tile_types[1].pieces.len(), 2);
    }

    #[test]
    fn tile_types_are_added_from_the_missing_client_types() {
        let mut pieces = sample();

        assert_eq!(add_tile_type(&mut pieces), HousingSelection::TileType { entry: 2 });
        assert_eq!(pieces.tile_types[2].tile_type, *HOUSING_CLIENT_TILE_TYPES.start() + 1);

        let mut full = HousingPieces {
            tile_types: HOUSING_CLIENT_TILE_TYPES
                .map(|tile_type| HousingTileType { tile_type, pieces: Vec::new() })
                .collect(),
        };
        add_tile_type(&mut full);
        assert_eq!(full.tile_types.last().unwrap().tile_type, HOUSING_CLIENT_TILE_TYPES.end() + 1);

        assert_eq!(remove_tile_type(&mut pieces, 2), Some(HousingSelection::TileType { entry: 1 }));
        assert_eq!(remove_tile_type(&mut pieces, 2), None);
    }

    #[test]
    fn slots_are_added_after_the_highest_and_removed_by_row() {
        let mut piece = sample().tile_types[0].pieces[0].clone();

        assert_eq!(add_slot(&mut piece, HousingSlotList::Solid), 1);
        assert_eq!(piece.solid[1], HousingSlot { slot: 2, id: 0 });
        assert_eq!(add_slot(&mut piece, HousingSlotList::Window), 0);
        assert_eq!(piece.windows[0].slot, HOUSING_FIRST_SLOT);

        assert!(remove_slot(&mut piece, HousingSlotList::Solid, 0));
        assert_eq!(piece.solid, [HousingSlot { slot: 2, id: 0 }]);
        assert!(!remove_slot(&mut piece, HousingSlotList::Multi, 1));
    }

    #[test]
    fn selections_resolve_to_existing_items() {
        let pieces = sample();

        assert_eq!(HousingSelection::Piece { entry: 1, piece: 1 }.resolve(&pieces), Some(HousingSelection::Piece { entry: 1, piece: 1 }));
        assert_eq!(HousingSelection::Piece { entry: 1, piece: 5 }.resolve(&pieces), Some(HousingSelection::TileType { entry: 1 }));
        assert_eq!(
            HousingSelection::Category { entry: 1, category: 0 }.resolve(&pieces),
            Some(HousingSelection::TileType { entry: 1 })
        );
        assert_eq!(HousingSelection::TileType { entry: 2 }.resolve(&pieces), None);
    }

    #[test]
    fn issues_select_their_piece_category_or_tile_type() {
        let mut pieces = sample();
        pieces.tile_types[1].pieces[1].feature_mask = HOUSING_FEATURE_AOW;
        let none = |_| false;
        let context = HousingValidationContext { tile_exists: Some(&none), ..Default::default() };

        let issues = pieces.validate(&context);

        let missing_window = issues
            .iter()
            .find(|issue| matches!(issue, HousingIssue::MissingTile { tile_id: WINDOW_TILE, .. }))
            .unwrap();
        assert_eq!(missing_window.severity(), HousingIssueSeverity::Error);
        let target = issue_target(missing_window);
        assert_eq!(
            target,
            HousingIssueTarget::Piece {
                key: HousingPieceKey { tile_type: HOUSING_TILE_TYPE_WALL, category: WALL_CATEGORY, group: 1 },
                slot: Some((HousingSlotList::Window, 1)),
            }
        );
        assert_eq!(find_target(&pieces, target), Some(HousingSelection::Piece { entry: 1, piece: 1 }));

        let mixed = issues.iter().find(|issue| matches!(issue, HousingIssue::MixedFeatureMask { .. })).unwrap();
        assert_eq!(find_target(&pieces, issue_target(mixed)), Some(HousingSelection::Category { entry: 1, category: WALL_CATEGORY }));

        let gap = HousingIssue::CategoryGap { tile_type: HOUSING_TILE_TYPE_WALL, category: 1 };
        assert_eq!(find_target(&pieces, issue_target(&gap)), Some(HousingSelection::TileType { entry: 1 }));

        let slot = HousingSlotRef {
            piece: HousingPieceKey { tile_type: HOUSING_TILE_TYPE_FLOOR, category: 0, group: 0 },
            list: HousingSlotList::Solid,
            slot: 1,
        };
        let absent = HousingIssue::MissingTile { slot, tile_id: WALL_TILE };
        assert_eq!(find_target(&pieces, issue_target(&absent)), None, "no floor tile type");

        // A repeated tile type: the client keeps the last entry.
        pieces.tile_types.push(pieces.tile_types[0].clone());
        let duplicate = HousingIssue::DuplicateTileType { tile_type: HOUSING_TILE_TYPE_STAIRS };
        assert_eq!(find_target(&pieces, issue_target(&duplicate)), Some(HousingSelection::TileType { entry: 2 }));
        assert_eq!(kept_entry(&pieces, HOUSING_TILE_TYPE_STAIRS), Some(2));
    }

    #[test]
    fn parses_decimal_and_hex_ids() {
        assert_eq!(parse_id(" 1817 "), Some(1817));
        assert_eq!(parse_id("0x071F"), Some(0x071F));
        assert_eq!(parse_id("0X71f"), Some(0x071F));
        assert_eq!(parse_id(""), None);
        assert_eq!(parse_id("wall"), None);
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let base = std::env::var_os(SCRATCH_DIR_ENV).map_or_else(std::env::temp_dir, PathBuf::from);
        let dir = base.join(format!("uocf_inspector_housing_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saves_and_reopens_every_format() {
        const MULTI_PAYLOAD: &[u8] = b"multi payload";
        let dir = scratch_dir("formats");
        let source = dir.join("MultiCollection.uop");
        let mut package = UopPackage::new_default();
        package.add_file_from_memory(MULTI_PAYLOAD, &multi_collection_path(STAIRS_MULTI), CompressionFlag::None).unwrap();
        package.add_file_from_memory(&HousingPieces::default().to_payload_bytes(), HOUSING_PATH, CompressionFlag::Zlib).unwrap();
        package.finalize_and_save(&source).unwrap();
        let pieces = sample();

        let output = dir.join("edited.uop");
        save_multi_collection(&pieces, &source, &output).unwrap();
        let reopened = open_client_package(&output).unwrap();
        assert_eq!(reopened.pieces(), &pieces);
        assert_eq!(reopened.source(), &HousingDocumentSource::ClientPackage(output.clone()));
        let written = UopPackage::load(&output).unwrap();
        assert_eq!(written.get_file_by_name(&multi_collection_path(STAIRS_MULTI)).unwrap().unpack().unwrap(), MULTI_PAYLOAD);

        let error = save_multi_collection(&pieces, &source, &dir.join(".").join("MultiCollection.uop")).unwrap_err();
        assert!(error.contains("refusing to overwrite"), "{error}");
        assert_eq!(open_client_package(&source).unwrap().pieces(), &HousingPieces::default(), "the source is unchanged");

        let loose = dir.join("housing.bin");
        save_housing_bin(&pieces, &loose).unwrap();
        assert_eq!(open_housing_bin(&loose).unwrap().pieces(), &pieces);

        let kdl = dir.join("housing.kdl");
        export_kdl(&pieces, &kdl).unwrap();
        assert_eq!(import_kdl(&kdl).unwrap().pieces(), &pieces);

        std::fs::write(&kdl, "tile-type 1 {\n    piece category=0\n}\n").unwrap();
        let error = import_kdl(&kdl).unwrap_err();
        assert!(error.contains("line 2"), "KDL errors name the line: {error}");
        assert!(open_client_package(&loose).is_err());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "needs a real Enhanced Client: set UO_EC_CLIENT_DIR and run with --ignored"]
    fn edits_of_the_client_housing_round_trip_through_every_format() {
        let client_dir = PathBuf::from(std::env::var_os(EC_CLIENT_DIR_ENV).expect("set UO_EC_CLIENT_DIR"));
        let source = client_dir.join(uocf::enhanced::multis::MULTI_COLLECTION_UOP_NAME);
        let mut document = open_client_package(&source).unwrap();
        let original = document.pieces().clone();
        let selection = document.edit(None, |pieces| add_piece(pieces, 0, None)).unwrap();
        let HousingSelection::Piece { entry, piece } = selection else { panic!("a piece is selected") };
        document.edit(None, |pieces| add_slot(&mut pieces.tile_types[entry].pieces[piece], HousingSlotList::Solid));
        let edited = document.pieces().clone();
        let dir = scratch_dir("client");

        let output = dir.join("MultiCollection.uop");
        save_multi_collection(&edited, &source, &output).unwrap();
        assert_eq!(open_client_package(&output).unwrap().pieces(), &edited);
        save_housing_bin(&edited, &dir.join("housing.bin")).unwrap();
        assert_eq!(open_housing_bin(&dir.join("housing.bin")).unwrap().pieces(), &edited);
        export_kdl(&edited, &dir.join("housing.kdl")).unwrap();
        assert_eq!(import_kdl(&dir.join("housing.kdl")).unwrap().pieces(), &edited);
        assert!(save_multi_collection(&edited, &source, &source).is_err());
        assert_eq!(open_client_package(&source).unwrap().pieces(), &original, "the client file is unchanged");

        std::fs::remove_dir_all(dir).unwrap();
    }
}
