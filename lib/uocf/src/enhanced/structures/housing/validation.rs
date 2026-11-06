//! Consistency checks of [`HousingPieces`] against the client's piece table rules
//! (`uosa_housing.md` sections 3.1 to 3.5) and, optionally, against the tileart, multi and cliloc data.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;

use super::{
    HousingPieceKey, HousingPieces, HousingSlotList, HousingSlotRef, HousingTileType, TileTypeLabel,
    HOUSING_CLIENT_EXPANSION_FILTER, HOUSING_CLIENT_TILE_TYPES,
};

/// Optional lookups for [`HousingPieces::validate`]; a `None` lookup skips its check.
///
/// ```ignore
/// let tiles: HashSet<u32> = ...;
/// let tile_exists = |id| tiles.contains(&id);
/// let context = HousingValidationContext { tile_exists: Some(&tile_exists), ..Default::default() };
/// ```
#[derive(Clone, Copy, Default)]
pub struct HousingValidationContext<'a> {
    /// Whether a tile id has a tileart record (solid and window ids).
    pub tile_exists: Option<&'a dyn Fn(u32) -> bool>,
    /// Whether a multi id is in the `MultiCollection.uop` (multi ids).
    pub multi_exists: Option<&'a dyn Fn(u32) -> bool>,
    /// Whether a cliloc has a string (tooltip clilocs).
    pub cliloc_exists: Option<&'a dyn Fn(u32) -> bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HousingIssueSeverity {
    /// Valid for the client, but probably not what the author meant.
    Warning,
    /// The client drops a whole tile type, or a piece refers to a tile or multi that does not exist.
    Error,
}

/// A problem found by [`HousingPieces::validate`]. `Display` gives a one-line description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HousingIssue {
    /// A tile type occurs more than once; the client keeps only the last entry. Error.
    DuplicateTileType { tile_type: u32 },
    /// A tile type outside [`HOUSING_CLIENT_TILE_TYPES`]; the client ignores its pieces. Warning.
    UnknownTileType { tile_type: u32 },
    /// Several pieces share a key; the client merges their lists into one group. Warning.
    DuplicatePiece { piece: HousingPieceKey },
    /// A slot repeated within the merged list of a group; the client keeps the last id. Warning (the client data
    /// has one: misc category 25 group 1, solid slot 2).
    DuplicateSlot { slot: HousingSlotRef, ignored_id: u32 },
    /// A multi list outside stairs or a window list outside walls (see
    /// [`HousingSlotList::expected_tile_type`]). Warning.
    UnexpectedList { piece: HousingPieceKey, list: HousingSlotList },
    /// A solid or window tile id without a tileart record. Error.
    MissingTile { slot: HousingSlotRef, tile_id: u32 },
    /// A multi id missing from the `MultiCollection.uop`. Error.
    MissingMulti { slot: HousingSlotRef, multi_id: u32 },
    /// The pieces of a category have different feature masks; the client uses the last one. Warning.
    MixedFeatureMask { tile_type: u32, category: u32, effective_mask: u32 },
    /// The pieces of a category carry different nonzero tooltips; the client uses the first one. Warning.
    ConflictingTooltips { tile_type: u32, category: u32, effective_cliloc: u32 },
    /// No piece of a category has a nonzero tooltip: the category has no name. Warning.
    CategoryWithoutTooltip { tile_type: u32, category: u32 },
    /// A tooltip cliloc without a string. Warning.
    MissingTooltipCliloc { piece: HousingPieceKey, cliloc: u32 },
    /// A feature mask with bits outside [`HOUSING_CLIENT_EXPANSION_FILTER`]; with `hidden` it has no bit inside,
    /// and the client's piece queries skip the piece. Warning.
    MaskOutsideExpansionFilter { piece: HousingPieceKey, feature_mask: u32, hidden: bool },
    /// A category number between the lowest and highest of a tile type without pieces; the client still makes an
    /// empty row for it. Warning.
    CategoryGap { tile_type: u32, category: u32 },
}

impl HousingIssue {
    pub fn severity(&self) -> HousingIssueSeverity {
        match self {
            Self::DuplicateTileType { .. } | Self::MissingTile { .. }
            | Self::MissingMulti { .. } => HousingIssueSeverity::Error,
            Self::UnknownTileType { .. }
            | Self::DuplicatePiece { .. }
            | Self::DuplicateSlot { .. }
            | Self::UnexpectedList { .. }
            | Self::MixedFeatureMask { .. }
            | Self::ConflictingTooltips { .. }
            | Self::CategoryWithoutTooltip { .. }
            | Self::MissingTooltipCliloc { .. }
            | Self::MaskOutsideExpansionFilter { .. }
            | Self::CategoryGap { .. } => HousingIssueSeverity::Warning,
        }
    }
}

impl fmt::Display for HousingIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateTileType { tile_type } => {
                write!(f, "{} occurs more than once; the client keeps only the last", TileTypeLabel(*tile_type))
            }
            Self::UnknownTileType { tile_type } => write!(
                f,
                "tile type {tile_type} is outside {}..={}; the client ignores it",
                HOUSING_CLIENT_TILE_TYPES.start(),
                HOUSING_CLIENT_TILE_TYPES.end()
            ),
            Self::DuplicatePiece { piece } => {
                write!(f, "{piece}: more than one piece; the client merges their lists")
            }
            Self::DuplicateSlot { slot, ignored_id } => {
                write!(f, "{slot}: repeated slot; the client ignores the earlier id {ignored_id:#06X}")
            }
            Self::UnexpectedList { piece, list } => {
                let expected = list.expected_tile_type().map(TileTypeLabel);
                match expected {
                    Some(expected) => write!(f, "{piece}: {} list outside {expected}", list.name()),
                    None => write!(f, "{piece}: unexpected {} list", list.name()),
                }
            }
            Self::MissingTile { slot, tile_id } => write!(f, "{slot}: tile {tile_id:#06X} has no tileart record"),
            Self::MissingMulti { slot, multi_id } => {
                write!(f, "{slot}: multi {multi_id} is not in the multi collection")
            }
            Self::MixedFeatureMask { tile_type, category, effective_mask } => write!(
                f,
                "{} category {category}: the pieces have different feature masks; the client uses the last, \
                 {effective_mask:#X}",
                TileTypeLabel(*tile_type)
            ),
            Self::ConflictingTooltips { tile_type, category, effective_cliloc } => write!(
                f,
                "{} category {category}: the pieces have different tooltips; the client uses the first, \
                 {effective_cliloc}",
                TileTypeLabel(*tile_type)
            ),
            Self::CategoryWithoutTooltip { tile_type, category } => {
                write!(f, "{} category {category}: no piece has a tooltip", TileTypeLabel(*tile_type))
            }
            Self::MissingTooltipCliloc { piece, cliloc } => write!(f, "{piece}: tooltip cliloc {cliloc} has no string"),
            Self::MaskOutsideExpansionFilter { piece, feature_mask, hidden } => {
                write!(
                    f,
                    "{piece}: feature mask {feature_mask:#X} has bits outside the client expansion filter \
                     {HOUSING_CLIENT_EXPANSION_FILTER:#X}"
                )?;
                if *hidden {
                    f.write_str(" and none inside: the client skips the piece")?;
                }
                Ok(())
            }
            Self::CategoryGap { tile_type, category } => write!(
                f,
                "{} category {category} has no pieces; the client shows an empty row",
                TileTypeLabel(*tile_type)
            ),
        }
    }
}

impl HousingPieces {
    /// Checks the pieces against the client's rules and the lookups of `context`. Issues are reported per tile
    /// type entry in file order; a repeated tile type is checked entry by entry.
    pub fn validate(&self, context: &HousingValidationContext<'_>) -> Vec<HousingIssue> {
        let mut issues = Vec::new();
        let mut seen_tile_types = HashSet::new();
        for entry in &self.tile_types {
            if !seen_tile_types.insert(entry.tile_type) {
                issues.push(HousingIssue::DuplicateTileType { tile_type: entry.tile_type });
            }
            if !HOUSING_CLIENT_TILE_TYPES.contains(&entry.tile_type) {
                issues.push(HousingIssue::UnknownTileType { tile_type: entry.tile_type });
            }
            validate_pieces(entry, context, &mut issues);
            validate_groups(entry, &mut issues);
            validate_categories(entry, &mut issues);
        }
        issues
    }
}

/// Checks of single pieces: list kinds, masks and the context lookups.
fn validate_pieces(entry: &HousingTileType, context: &HousingValidationContext<'_>, issues: &mut Vec<HousingIssue>) {
    for piece in &entry.pieces {
        let key = piece.key(entry.tile_type);
        for list in HousingSlotList::ALL {
            let slots = piece.slots(list);
            if slots.is_empty() {
                continue;
            }
            if list.expected_tile_type().is_some_and(|expected| expected != entry.tile_type) {
                issues.push(HousingIssue::UnexpectedList { piece: key, list });
            }
            let lookup = if list.holds_tile_ids() { context.tile_exists } else { context.multi_exists };
            let Some(exists) = lookup else {
                continue;
            };
            for slot in slots.iter().filter(|slot| !exists(slot.id)) {
                let slot_ref = HousingSlotRef { piece: key, list, slot: slot.slot };
                issues.push(if list.holds_tile_ids() {
                    HousingIssue::MissingTile { slot: slot_ref, tile_id: slot.id }
                } else {
                    HousingIssue::MissingMulti { slot: slot_ref, multi_id: slot.id }
                });
            }
        }

        let outside_filter = piece.feature_mask & !HOUSING_CLIENT_EXPANSION_FILTER;
        let hidden = piece.feature_mask & HOUSING_CLIENT_EXPANSION_FILTER == 0;
        if outside_filter != 0 || hidden {
            let feature_mask = piece.feature_mask;
            issues.push(HousingIssue::MaskOutsideExpansionFilter { piece: key, feature_mask, hidden });
        }

        if piece.tooltip_cliloc != 0 && context.cliloc_exists.is_some_and(|exists| !exists(piece.tooltip_cliloc)) {
            issues.push(HousingIssue::MissingTooltipCliloc { piece: key, cliloc: piece.tooltip_cliloc });
        }
    }
}

/// Checks of the client groups, keyed by (category, group): repeated keys and the slots of the merged lists.
fn validate_groups(entry: &HousingTileType, issues: &mut Vec<HousingIssue>) {
    let mut groups = BTreeMap::<(u32, u32), Vec<_>>::new();
    for piece in &entry.pieces {
        groups.entry((piece.category, piece.group)).or_default().push(piece);
    }
    for pieces in groups.values() {
        let key = pieces[0].key(entry.tile_type);
        if pieces.len() > 1 {
            issues.push(HousingIssue::DuplicatePiece { piece: key });
        }
        for list in HousingSlotList::ALL {
            let mut ids_by_slot = BTreeMap::new();
            for slot in pieces.iter().flat_map(|piece| piece.slots(list)) {
                if let Some(ignored_id) = ids_by_slot.insert(slot.slot, slot.id) {
                    let slot = HousingSlotRef { piece: key, list, slot: slot.slot };
                    issues.push(HousingIssue::DuplicateSlot { slot, ignored_id });
                }
            }
        }
    }
}

/// Checks of the categories (window rows): feature masks, tooltips and gaps.
fn validate_categories(entry: &HousingTileType, issues: &mut Vec<HousingIssue>) {
    let mut categories = BTreeMap::<u32, Vec<_>>::new();
    for piece in &entry.pieces {
        categories.entry(piece.category).or_default().push(piece);
    }
    let tile_type = entry.tile_type;
    for (&category, pieces) in &categories {
        let effective_mask = pieces[pieces.len() - 1].feature_mask;
        if pieces.iter().any(|piece| piece.feature_mask != effective_mask) {
            issues.push(HousingIssue::MixedFeatureMask { tile_type, category, effective_mask });
        }

        let mut tooltips = pieces.iter().map(|piece| piece.tooltip_cliloc).filter(|&cliloc| cliloc != 0);
        match tooltips.next() {
            None => issues.push(HousingIssue::CategoryWithoutTooltip { tile_type, category }),
            Some(effective_cliloc) => {
                if tooltips.any(|cliloc| cliloc != effective_cliloc) {
                    issues.push(HousingIssue::ConflictingTooltips { tile_type, category, effective_cliloc });
                }
            }
        }
    }

    let present: BTreeSet<u32> = categories.keys().copied().collect();
    if let (Some(&first), Some(&last)) = (present.first(), present.last()) {
        for category in (first..=last).filter(|category| !present.contains(category)) {
            issues.push(HousingIssue::CategoryGap { tile_type, category });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::housing::{
        HousingPiece, HousingSlot, HOUSING_FEATURE_AOW, HOUSING_FEATURE_NO_EXPANSION, HOUSING_TILE_TYPE_FLOOR,
        HOUSING_TILE_TYPE_STAIRS, HOUSING_TILE_TYPE_WALL,
    };

    const TOOLTIP: u32 = 1061019;
    const OTHER_TOOLTIP: u32 = 1061020;
    const TILE_ID: u32 = 0x0009;
    const OTHER_TILE_ID: u32 = 0x000A;
    const MULTI_ID: u32 = 7600;
    const UNKNOWN_TILE_TYPE: u32 = 9;
    const GAP_CATEGORY: u32 = 1;
    const LAST_CATEGORY: u32 = 2;
    /// A feature bit the client expansion filter does not include.
    const BIT_OUTSIDE_FILTER: u32 = 0x400;

    fn piece(category: u32, group: u32) -> HousingPiece {
        HousingPiece {
            category,
            group,
            feature_mask: HOUSING_FEATURE_NO_EXPANSION,
            tooltip_cliloc: TOOLTIP,
            solid: vec![HousingSlot { slot: 1, id: TILE_ID }],
            multis: Vec::new(),
            windows: Vec::new(),
        }
    }

    fn pieces_of(tile_type: u32, pieces: Vec<HousingPiece>) -> HousingPieces {
        HousingPieces { tile_types: vec![HousingTileType { tile_type, pieces }] }
    }

    fn issues(pieces: &HousingPieces) -> Vec<HousingIssue> {
        pieces.validate(&HousingValidationContext::default())
    }

    fn key(tile_type: u32, category: u32, group: u32) -> HousingPieceKey {
        HousingPieceKey { tile_type, category, group }
    }

    #[test]
    fn clean_pieces_have_no_issues() {
        let mut stairs = piece(0, 0);
        stairs.multis.push(HousingSlot { slot: 1, id: MULTI_ID });
        let pieces = HousingPieces {
            tile_types: vec![
                HousingTileType { tile_type: HOUSING_TILE_TYPE_STAIRS, pieces: vec![stairs] },
                HousingTileType { tile_type: HOUSING_TILE_TYPE_FLOOR, pieces: vec![piece(0, 0), piece(1, 0)] },
            ],
        };
        let all = |_| true;
        let context =
            HousingValidationContext { tile_exists: Some(&all), multi_exists: Some(&all), cliloc_exists: Some(&all) };

        assert_eq!(pieces.validate(&context), []);
    }

    #[test]
    fn reports_duplicate_and_unknown_tile_types() {
        let mut pieces = pieces_of(HOUSING_TILE_TYPE_FLOOR, vec![piece(0, 0)]);
        pieces.tile_types.push(pieces.tile_types[0].clone());
        pieces.tile_types.push(HousingTileType { tile_type: UNKNOWN_TILE_TYPE, pieces: vec![piece(0, 0)] });

        let found = issues(&pieces);

        assert_eq!(
            found,
            [
                HousingIssue::DuplicateTileType { tile_type: HOUSING_TILE_TYPE_FLOOR },
                HousingIssue::UnknownTileType { tile_type: UNKNOWN_TILE_TYPE },
            ]
        );
        assert_eq!(found[0].severity(), HousingIssueSeverity::Error);
        assert_eq!(found[1].severity(), HousingIssueSeverity::Warning);
    }

    #[test]
    fn reports_duplicate_pieces_and_slots_of_the_merged_group() {
        let mut second = piece(0, 0);
        second.solid = vec![HousingSlot { slot: 1, id: OTHER_TILE_ID }, HousingSlot { slot: 2, id: TILE_ID }];
        let pieces = pieces_of(HOUSING_TILE_TYPE_FLOOR, vec![piece(0, 0), second]);

        let found = issues(&pieces);

        let slot = HousingSlotRef { piece: key(HOUSING_TILE_TYPE_FLOOR, 0, 0), list: HousingSlotList::Solid, slot: 1 };
        assert_eq!(
            found,
            [
                HousingIssue::DuplicatePiece { piece: key(HOUSING_TILE_TYPE_FLOOR, 0, 0) },
                HousingIssue::DuplicateSlot { slot, ignored_id: TILE_ID },
            ]
        );
        assert_eq!(found[1].severity(), HousingIssueSeverity::Warning);
    }

    #[test]
    fn reports_lists_outside_their_tile_type() {
        let mut floor = piece(0, 0);
        floor.multis.push(HousingSlot { slot: 1, id: MULTI_ID });
        floor.windows.push(HousingSlot { slot: 1, id: TILE_ID });
        let mut wall = piece(0, 0);
        wall.windows.push(HousingSlot { slot: 1, id: TILE_ID });
        let mut pieces = pieces_of(HOUSING_TILE_TYPE_FLOOR, vec![floor]);
        pieces.tile_types.push(HousingTileType { tile_type: HOUSING_TILE_TYPE_WALL, pieces: vec![wall] });

        let piece_key = key(HOUSING_TILE_TYPE_FLOOR, 0, 0);
        assert_eq!(
            issues(&pieces),
            [
                HousingIssue::UnexpectedList { piece: piece_key, list: HousingSlotList::Multi },
                HousingIssue::UnexpectedList { piece: piece_key, list: HousingSlotList::Window },
            ]
        );
    }

    #[test]
    fn reports_missing_tiles_multis_and_clilocs_from_the_context() {
        let mut stairs = piece(0, 0);
        stairs.solid.push(HousingSlot { slot: 2, id: OTHER_TILE_ID });
        stairs.multis.push(HousingSlot { slot: 1, id: MULTI_ID });
        let pieces = pieces_of(HOUSING_TILE_TYPE_STAIRS, vec![stairs]);
        let tile_exists = |id| id == TILE_ID;
        let none = |_| false;
        let context = HousingValidationContext {
            tile_exists: Some(&tile_exists),
            multi_exists: Some(&none),
            cliloc_exists: Some(&none),
        };

        let found = pieces.validate(&context);

        let piece_key = key(HOUSING_TILE_TYPE_STAIRS, 0, 0);
        assert_eq!(
            found,
            [
                HousingIssue::MissingTile {
                    slot: HousingSlotRef { piece: piece_key, list: HousingSlotList::Solid, slot: 2 },
                    tile_id: OTHER_TILE_ID,
                },
                HousingIssue::MissingMulti {
                    slot: HousingSlotRef { piece: piece_key, list: HousingSlotList::Multi, slot: 1 },
                    multi_id: MULTI_ID,
                },
                HousingIssue::MissingTooltipCliloc { piece: piece_key, cliloc: TOOLTIP },
            ]
        );
        assert_eq!(found[2].severity(), HousingIssueSeverity::Warning);
        assert!(issues(&pieces).is_empty(), "lookups not given are not checked");
    }

    #[test]
    fn reports_mixed_feature_masks_with_the_last_one() {
        let mut last = piece(0, 1);
        last.feature_mask = HOUSING_FEATURE_AOW;
        let pieces = pieces_of(HOUSING_TILE_TYPE_WALL, vec![piece(0, 0), last]);

        assert_eq!(
            issues(&pieces),
            [HousingIssue::MixedFeatureMask {
                tile_type: HOUSING_TILE_TYPE_WALL,
                category: 0,
                effective_mask: HOUSING_FEATURE_AOW,
            }]
        );
    }

    #[test]
    fn reports_category_tooltips_missing_or_conflicting() {
        let mut no_tooltip = piece(1, 0);
        no_tooltip.tooltip_cliloc = 0;
        let mut other_tooltip = piece(0, 1);
        other_tooltip.tooltip_cliloc = OTHER_TOOLTIP;
        let mut same_tooltip = piece(0, 2);
        same_tooltip.tooltip_cliloc = TOOLTIP;
        let pieces = pieces_of(HOUSING_TILE_TYPE_WALL, vec![piece(0, 0), other_tooltip, same_tooltip, no_tooltip]);

        assert_eq!(
            issues(&pieces),
            [
                HousingIssue::ConflictingTooltips {
                    tile_type: HOUSING_TILE_TYPE_WALL,
                    category: 0,
                    effective_cliloc: TOOLTIP,
                },
                HousingIssue::CategoryWithoutTooltip { tile_type: HOUSING_TILE_TYPE_WALL, category: 1 },
            ]
        );
    }

    #[test]
    fn reports_masks_outside_the_expansion_filter() {
        let mut extra_bit = piece(0, 0);
        extra_bit.feature_mask = HOUSING_FEATURE_NO_EXPANSION | BIT_OUTSIDE_FILTER;
        let mut hidden = piece(1, 0);
        hidden.feature_mask = BIT_OUTSIDE_FILTER;
        let pieces = pieces_of(HOUSING_TILE_TYPE_FLOOR, vec![extra_bit, hidden]);

        let found = issues(&pieces);

        assert_eq!(
            found,
            [
                HousingIssue::MaskOutsideExpansionFilter {
                    piece: key(HOUSING_TILE_TYPE_FLOOR, 0, 0),
                    feature_mask: HOUSING_FEATURE_NO_EXPANSION | BIT_OUTSIDE_FILTER,
                    hidden: false,
                },
                HousingIssue::MaskOutsideExpansionFilter {
                    piece: key(HOUSING_TILE_TYPE_FLOOR, 1, 0),
                    feature_mask: BIT_OUTSIDE_FILTER,
                    hidden: true,
                },
            ]
        );
        assert!(found[1].to_string().contains("skips the piece"), "{}", found[1]);
    }

    #[test]
    fn reports_category_gaps() {
        let pieces = pieces_of(HOUSING_TILE_TYPE_FLOOR, vec![piece(0, 0), piece(LAST_CATEGORY, 0)]);

        assert_eq!(
            issues(&pieces),
            [HousingIssue::CategoryGap { tile_type: HOUSING_TILE_TYPE_FLOOR, category: GAP_CATEGORY }]
        );
    }

    #[test]
    fn issue_messages_name_the_location() {
        let slot = HousingSlotRef { piece: key(HOUSING_TILE_TYPE_STAIRS, 3, 0), list: HousingSlotList::Solid, slot: 9 };

        assert_eq!(
            HousingIssue::MissingTile { slot, tile_id: TILE_ID }.to_string(),
            "stairs category 3 group 0 solid slot 9: tile 0x0009 has no tileart record"
        );
    }
}
