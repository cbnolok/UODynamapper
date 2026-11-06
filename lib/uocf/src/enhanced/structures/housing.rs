//! Decoding for `build/multicollection/housing.bin` in `MultiCollection.uop`: the piece tables of the house
//! customization tool.
//!
//! Layout and field meanings follow the client parser (`UOCustomHousingPieces::Parse` in UOSA.exe) and its
//! only reader (`UOHouseCustomization::BuildPieceTables`), documented in
//! `scripts/dev-helpers/ec-research/ghidra-housing/results/uosa_housing.md` (sections 2 and 3). The asset is
//! separate from the multi entries of the same package; the only link is the stairs multi list, which refers to
//! the stair block multis.
//!
//! Besides decoding, the module serves an editor: [`HousingPieces::to_payload_bytes`] is the exact inverse of the
//! parser, [`HousingPieces::validate`] reports what the client would drop or misread, the category helpers
//! ([`HousingPieces::category_entitlement`], [`HousingPieces::category_tooltip`], [`HousingPieces::tile_index`])
//! follow the client's piece table rules, and the pieces can be written as a loose `housing.bin`, as a copy of a
//! `MultiCollection.uop` with only `housing.bin` replaced, or as KDL text ([`HousingPieces::to_kdl_string`],
//! schema in `housing/kdl_text.rs`).

mod kdl_text;
mod validation;

pub use validation::{HousingIssue, HousingIssueSeverity, HousingValidationContext};

use std::collections::BTreeMap;
use std::fmt;
use std::io::Cursor;
use std::ops::RangeInclusive;
use std::path::Path;

use byteorder::{LittleEndian, ReadBytesExt};
use color_eyre::eyre::{self, WrapErr};

use super::multis::HOUSING_PATH;
use crate::uop_container::hash::hash_file_name_single;
use crate::uop_container::package::UopPackage;

// Tile types (Lua `CustomizeHouseWindow` constants). The client visits 1..=7. Confidence: high.
pub const HOUSING_TILE_TYPE_STAIRS: u32 = 1;
pub const HOUSING_TILE_TYPE_ROOF: u32 = 2;
pub const HOUSING_TILE_TYPE_DOOR: u32 = 3;
pub const HOUSING_TILE_TYPE_FLOOR: u32 = 4;
pub const HOUSING_TILE_TYPE_WALL: u32 = 5;
pub const HOUSING_TILE_TYPE_TELEPORTER: u32 = 6;
pub const HOUSING_TILE_TYPE_MISC: u32 = 7;
/// Tile types the client builds piece tables for (`BuildPieceTables`); pieces of other tile types are ignored.
pub const HOUSING_CLIENT_TILE_TYPES: RangeInclusive<u32> = HOUSING_TILE_TYPE_STAIRS..=HOUSING_TILE_TYPE_MISC;

// Feature mask bits of a piece, compared with the 0xB9 server feature flags. Expansion names come from the
// client's `HousingPieceFilter::Describe` (confidence: medium), the theme pack names from the texts of their
// entitlement clilocs.

/// No purchased expansion: always granted.
pub const HOUSING_FEATURE_NO_EXPANSION: u32 = 0x1;
pub const HOUSING_FEATURE_AOW: u32 = 0x40;
pub const HOUSING_FEATURE_EP1: u32 = 0x80;
/// Named by the client but not used by any piece in the current data.
pub const HOUSING_FEATURE_8TH: u32 = 0x100;
pub const HOUSING_FEATURE_9TH: u32 = 0x200;
pub const HOUSING_FEATURE_SA: u32 = 0x10000;
pub const HOUSING_FEATURE_GOTHIC_THEME: u32 = 0x40000;
pub const HOUSING_FEATURE_RUSTIC_THEME: u32 = 0x80000;
pub const HOUSING_FEATURE_TIME_OF_LEGENDS: u32 = 0x400000;
/// Client expansion filter: piece queries ignore pieces whose mask has no bit in it (every current piece passes).
pub const HOUSING_CLIENT_EXPANSION_FILTER: u32 = 0x6f3bff;

/// The known feature mask bits with their names, in bit order: the `HousingPieceFilter::Describe` texts for the
/// expansions, the entitlement cliloc texts for the theme packs and Time of Legends.
pub const HOUSING_FEATURES: [(u32, &str); 9] = [
    (HOUSING_FEATURE_NO_EXPANSION, "no purchased expansions"),
    (HOUSING_FEATURE_AOW, "AOW"),
    (HOUSING_FEATURE_EP1, "EP1"),
    (HOUSING_FEATURE_8TH, "8th"),
    (HOUSING_FEATURE_9TH, "9th"),
    (HOUSING_FEATURE_SA, "SA"),
    (HOUSING_FEATURE_GOTHIC_THEME, "Gothic Theme Pack"),
    (HOUSING_FEATURE_RUSTIC_THEME, "Rustic Theme Pack"),
    (HOUSING_FEATURE_TIME_OF_LEGENDS, "Time of Legends"),
];

// Entitlement clilocs (`entitlementTID`) the client picks from the feature mask of a category (the mask of its
// last piece). Confidence: high.

/// "You must upgrade to the latest Expansion Pack ...": every mask except the theme packs and Time of Legends.
pub const HOUSING_ENTITLEMENT_CLILOC_UPGRADE: u32 = 1062930;
pub const HOUSING_ENTITLEMENT_CLILOC_GOTHIC_THEME: u32 = 1150774;
pub const HOUSING_ENTITLEMENT_CLILOC_RUSTIC_THEME: u32 = 1150773;
pub const HOUSING_ENTITLEMENT_CLILOC_TIME_OF_LEGENDS: u32 = 1156114;

/// Labels of the stairs Solid slots 1..=9: the classic `stairs.txt` columns North..Rounded2, then Block (the
/// classic column order is rotated by one). Confidence: medium (row-by-row comparison with the classic file).
const STAIRS_SOLID_SLOT_LABELS: [&str; 9] =
    ["North", "East", "South", "West", "Squared1", "Squared2", "Rounded1", "Rounded2", "Block"];
/// Labels of the stairs Multi slots 1..=4: the classic `stairs.txt` columns MultiNorth..MultiWest.
/// Confidence: medium.
const STAIRS_MULTI_SLOT_LABELS: [&str; 4] = ["MultiNorth", "MultiEast", "MultiSouth", "MultiWest"];
/// Slots of the labelled lists start at 1.
const FIRST_LABELLED_SLOT: u32 = 1;

/// Bytes of a tile type header: tile type, piece count.
const TILE_TYPE_HEADER_SIZE: usize = 2 * size_of::<u32>();
/// Bytes of a piece header: category, group, feature mask, tooltip cliloc.
const PIECE_HEADER_SIZE: usize = 4 * size_of::<u32>();
/// Bytes of a slot entry: slot, id.
const SLOT_SIZE: usize = 2 * size_of::<u32>();

/// Decoded `housing.bin`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HousingPieces {
    /// Tile types in file order. The client keeps them in a map keyed by tile type, where a repeated tile type
    /// replaces the earlier one; the current data has each of 1..=7 once.
    pub tile_types: Vec<HousingTileType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HousingTileType {
    /// One of the `HOUSING_TILE_TYPE_*` constants.
    pub tile_type: u32,
    pub pieces: Vec<HousingPiece>,
}

/// One piece (client piece, 0x40 bytes). The lists are stored in the file in the order solid, multi, window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HousingPiece {
    /// Row of the customization window within the tile type (the "style"). Lua publishes it 1-based.
    /// Confidence: high.
    pub category: u32,
    /// Sub-row of the category (only walls and misc use groups other than 0). Lua publishes it 1-based.
    /// Confidence: high.
    pub group: u32,
    /// `HOUSING_FEATURE_*` bits. The client uses the mask of the last piece of a category for its entitlement.
    pub feature_mask: u32,
    /// Category name cliloc; the client uses the nonzero tooltip of the category's pieces. Confidence: high.
    pub tooltip_cliloc: u32,
    /// Tile ids of the piece variants (wall orientations, roof pieces, floor tiles, door states, stair parts).
    pub solid: Vec<HousingSlot>,
    /// Stair block multi ids (7600..=7675, slots 1..=4 = N/E/S/W); stairs only.
    pub multis: Vec<HousingSlot>,
    /// Wall variants with windows; walls only.
    pub windows: Vec<HousingSlot>,
}

/// One list entry. The client keys the entries of a group by slot (last write wins); the slot is the tile
/// subtype of the selection. Confidence: high for the storage, medium for the per-type slot meanings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HousingSlot {
    pub slot: u32,
    /// Tile id for the solid and window lists, multi id for the multi list.
    pub id: u32,
}

/// The three slot lists of a piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HousingSlotList {
    Solid,
    Multi,
    Window,
}

impl HousingSlotList {
    /// The lists in file order.
    pub const ALL: [Self; 3] = [Self::Solid, Self::Multi, Self::Window];

    /// Lowercase name, also the KDL node name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Multi => "multi",
            Self::Window => "window",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|list| list.name() == name)
    }

    /// True for the lists of tile ids (solid, window), false for the multi id list.
    pub fn holds_tile_ids(self) -> bool {
        self != Self::Multi
    }

    /// The only tile type whose pieces use the list (multi: stairs, window: walls); `None` for the solid list.
    /// The client reads the lists of every piece; the client selects any multi as a stairs piece.
    pub fn expected_tile_type(self) -> Option<u32> {
        match self {
            Self::Solid => None,
            Self::Multi => Some(HOUSING_TILE_TYPE_STAIRS),
            Self::Window => Some(HOUSING_TILE_TYPE_WALL),
        }
    }
}

/// The client key of a piece: tile type, category (window row) and group (sub-row).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HousingPieceKey {
    pub tile_type: u32,
    pub category: u32,
    pub group: u32,
}

impl fmt::Display for HousingPieceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} category {} group {}", TileTypeLabel(self.tile_type), self.category, self.group)
    }
}

/// One slot of one list of a piece, e.g. an entry of [`HousingPieces::tile_index`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HousingSlotRef {
    pub piece: HousingPieceKey,
    pub list: HousingSlotList,
    pub slot: u32,
}

impl fmt::Display for HousingSlotRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} slot {}", self.piece, self.list.name(), self.slot)
    }
}

/// The entitlement the client gives a category from its feature mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HousingEntitlement {
    /// The mask of the category's last piece; the category is granted when the 0xB9 feature flags share a bit
    /// with it.
    pub feature_mask: u32,
    /// Name of the mask when it is a single known bit ([`HOUSING_FEATURES`]).
    pub expansion: Option<&'static str>,
    /// `entitlementTID`: the cliloc shown when the category is not granted.
    pub cliloc: u32,
}

/// How well a field meaning is established.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HousingConfidence {
    Low,
    Medium,
    High,
}

/// Meaning of a slot of a list for a tile type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HousingSlotLabel {
    pub label: &'static str,
    pub confidence: HousingConfidence,
}

impl HousingPieces {
    /// Loads `housing.bin` from a `MultiCollection.uop` package; `None` when the package has no such entry.
    pub fn load(path: impl AsRef<Path>) -> eyre::Result<Option<Self>> {
        let package = UopPackage::load(path)?;
        Self::from_package(&package)
    }

    /// Decodes `housing.bin` from a loaded `MultiCollection.uop`; `None` when the package has no such entry.
    pub fn from_package(package: &UopPackage) -> eyre::Result<Option<Self>> {
        let Some(file) = package.get_file_by_hash(hash_file_name_single(HOUSING_PATH)) else {
            return Ok(None);
        };
        let bytes = file.unpack().wrap_err_with(|| format!("failed to unpack {HOUSING_PATH}"))?;
        Self::from_payload_bytes(&bytes).map(Some)
    }

    /// Decodes a `housing.bin` payload: u32 tile type count, then per tile type a u32 tile type and a u32 piece
    /// count, then the pieces `{u32 category, u32 group, u32 feature mask, u32 tooltip cliloc, solid list,
    /// multi list, window list}`, each list a u32 count of `{u32 slot, u32 id}`. As in the client, the payload
    /// must end exactly after the last tile type.
    pub fn from_payload_bytes(bytes: &[u8]) -> eyre::Result<Self> {
        let mut reader = Cursor::new(bytes);
        let tile_type_count = read_u32(&mut reader, "tile type count")? as usize;
        let mut tile_types = Vec::with_capacity(tile_type_count.min(remaining_len(&reader) / TILE_TYPE_HEADER_SIZE));
        for _ in 0..tile_type_count {
            let tile_type = read_u32(&mut reader, "tile type")?;
            let piece_count = read_u32(&mut reader, "piece count")? as usize;
            let mut pieces = Vec::with_capacity(piece_count.min(remaining_len(&reader) / PIECE_HEADER_SIZE));
            for _ in 0..piece_count {
                pieces.push(read_piece(&mut reader)?);
            }
            tile_types.push(HousingTileType { tile_type, pieces });
        }

        let trailing = remaining_len(&reader);
        if trailing != 0 {
            eyre::bail!("{trailing} trailing bytes after {tile_type_count} tile types in {HOUSING_PATH}");
        }
        Ok(Self { tile_types })
    }

    /// Loads a loose `housing.bin` file.
    pub fn load_housing_bin(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).wrap_err_with(|| format!("failed to read {}", path.display()))?;
        Self::from_payload_bytes(&bytes).wrap_err_with(|| format!("failed to decode {}", path.display()))
    }

    /// Encodes the `housing.bin` payload: the exact inverse of [`Self::from_payload_bytes`].
    ///
    /// # Panics
    /// When a count does not fit the u32 of the format.
    pub fn to_payload_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_count(&mut bytes, self.tile_types.len());
        for tile_type in &self.tile_types {
            push_u32(&mut bytes, tile_type.tile_type);
            push_count(&mut bytes, tile_type.pieces.len());
            for piece in &tile_type.pieces {
                for value in [piece.category, piece.group, piece.feature_mask, piece.tooltip_cliloc] {
                    push_u32(&mut bytes, value);
                }
                for list in HousingSlotList::ALL {
                    let slots = piece.slots(list);
                    push_count(&mut bytes, slots.len());
                    for slot in slots {
                        push_u32(&mut bytes, slot.slot);
                        push_u32(&mut bytes, slot.id);
                    }
                }
            }
        }
        bytes
    }

    /// Writes a loose `housing.bin` file.
    pub fn write_housing_bin(&self, path: impl AsRef<Path>) -> eyre::Result<()> {
        let path = path.as_ref();
        std::fs::write(path, self.to_payload_bytes()).wrap_err_with(|| format!("failed to write {}", path.display()))
    }

    /// Replaces the `housing.bin` entry of a `MultiCollection.uop` package with these pieces, keeping the
    /// entry's compression flag (`UopPackage::replace_file`): a changed entry gets a fresh timestamp data block
    /// header stamped with the package's edit FILETIME, an unchanged one is left untouched. Fails when the
    /// package has no `housing.bin`.
    pub fn replace_in_package(&self, package: &mut UopPackage) -> eyre::Result<()> {
        let hash = hash_file_name_single(HOUSING_PATH);
        let Some(file) = package.get_file_by_hash(hash) else {
            eyre::bail!("the package has no {HOUSING_PATH}");
        };
        let compression = file.compression();
        package
            .replace_file(hash, &self.to_payload_bytes(), compression)
            .wrap_err_with(|| format!("failed to replace {HOUSING_PATH}"))?;
        Ok(())
    }

    /// Writes a copy of the `source` `MultiCollection.uop` to `output` with only `housing.bin` replaced
    /// ([`Self::replace_in_package`], stamped with the current time): every other entry keeps its payload,
    /// compression flag and data block header, the package keeps its version and block size. `output` must not
    /// be `source`.
    pub fn write_multi_collection(&self, source: impl AsRef<Path>, output: impl AsRef<Path>) -> eyre::Result<()> {
        let (source, output) = (source.as_ref(), output.as_ref());
        let canonical_source = std::fs::canonicalize(source)
            .wrap_err_with(|| format!("failed to resolve {}", source.display()))?;
        if std::fs::canonicalize(output).is_ok_and(|canonical_output| canonical_output == canonical_source) {
            eyre::bail!("refusing to overwrite the source package {}: write to a new path", source.display());
        }

        let mut package =
            UopPackage::load(source).wrap_err_with(|| format!("failed to load {}", source.display()))?;
        self.replace_in_package(&mut package)
            .wrap_err_with(|| format!("failed to replace {HOUSING_PATH} in {}", source.display()))?;
        package.finalize_and_save(output).wrap_err_with(|| format!("failed to write {}", output.display()))
    }

    pub fn pieces(&self) -> impl Iterator<Item = (u32, &HousingPiece)> {
        self.tile_types
            .iter()
            .flat_map(|tile_type| tile_type.pieces.iter().map(|piece| (tile_type.tile_type, piece)))
    }

    /// The tile type entry the client keeps for `tile_type`: the last one, as a repeated tile type replaces the
    /// earlier ones.
    pub fn tile_type(&self, tile_type: u32) -> Option<&HousingTileType> {
        self.tile_types.iter().rev().find(|entry| entry.tile_type == tile_type)
    }

    /// The pieces of a category in file order, all groups (of the tile type entry the client keeps).
    pub fn category_pieces(&self, tile_type: u32, category: u32) -> impl DoubleEndedIterator<Item = &HousingPiece> {
        self.tile_type(tile_type)
            .into_iter()
            .flat_map(|entry| entry.pieces.iter())
            .filter(move |piece| piece.category == category)
    }

    /// The feature mask the client uses for a category: the mask of its last piece (any group).
    pub fn category_feature_mask(&self, tile_type: u32, category: u32) -> Option<u32> {
        self.category_pieces(tile_type, category).next_back().map(|piece| piece.feature_mask)
    }

    /// The entitlement of a category, from [`Self::category_feature_mask`].
    pub fn category_entitlement(&self, tile_type: u32, category: u32) -> Option<HousingEntitlement> {
        self.category_feature_mask(tile_type, category).map(housing_entitlement)
    }

    /// The category name cliloc: the first nonzero tooltip of the category's pieces (any group), as the client's
    /// `GetTooltip`.
    pub fn category_tooltip(&self, tile_type: u32, category: u32) -> Option<u32> {
        self.category_pieces(tile_type, category)
            .map(|piece| piece.tooltip_cliloc)
            .find(|&cliloc| cliloc != 0)
    }

    /// Every slot of the tile id lists (solid, window) by tile id, in file order, over all tile type entries.
    /// The client's lookup from a tile id to its tile type and slot keeps one of them per id.
    pub fn tile_index(&self) -> BTreeMap<u32, Vec<HousingSlotRef>> {
        let mut index = BTreeMap::<u32, Vec<HousingSlotRef>>::new();
        for (tile_type, piece) in self.pieces() {
            for list in HousingSlotList::ALL.into_iter().filter(|list| list.holds_tile_ids()) {
                for slot in piece.slots(list) {
                    index.entry(slot.id).or_default().push(HousingSlotRef {
                        piece: piece.key(tile_type),
                        list,
                        slot: slot.slot,
                    });
                }
            }
        }
        index
    }
}

impl HousingPiece {
    pub fn key(&self, tile_type: u32) -> HousingPieceKey {
        HousingPieceKey { tile_type, category: self.category, group: self.group }
    }

    pub fn slots(&self, list: HousingSlotList) -> &[HousingSlot] {
        match list {
            HousingSlotList::Solid => &self.solid,
            HousingSlotList::Multi => &self.multis,
            HousingSlotList::Window => &self.windows,
        }
    }

    pub fn slots_mut(&mut self, list: HousingSlotList) -> &mut Vec<HousingSlot> {
        match list {
            HousingSlotList::Solid => &mut self.solid,
            HousingSlotList::Multi => &mut self.multis,
            HousingSlotList::Window => &mut self.windows,
        }
    }
}

/// Name of a single feature mask bit ([`HOUSING_FEATURES`]).
pub fn housing_feature_name(bit: u32) -> Option<&'static str> {
    HOUSING_FEATURES.iter().find(|(feature, _)| *feature == bit).map(|(_, name)| *name)
}

/// The `entitlementTID` the client picks for a category feature mask: the theme pack and Time of Legends
/// clilocs for exactly their bit, the "upgrade to the latest Expansion Pack" cliloc otherwise.
pub fn housing_entitlement_cliloc(feature_mask: u32) -> u32 {
    match feature_mask {
        HOUSING_FEATURE_GOTHIC_THEME => HOUSING_ENTITLEMENT_CLILOC_GOTHIC_THEME,
        HOUSING_FEATURE_RUSTIC_THEME => HOUSING_ENTITLEMENT_CLILOC_RUSTIC_THEME,
        HOUSING_FEATURE_TIME_OF_LEGENDS => HOUSING_ENTITLEMENT_CLILOC_TIME_OF_LEGENDS,
        _ => HOUSING_ENTITLEMENT_CLILOC_UPGRADE,
    }
}

/// The entitlement of a category whose last piece has `feature_mask`.
pub fn housing_entitlement(feature_mask: u32) -> HousingEntitlement {
    HousingEntitlement {
        feature_mask,
        expansion: housing_feature_name(feature_mask),
        cliloc: housing_entitlement_cliloc(feature_mask),
    }
}

/// Meaning of a slot where it is known: the stairs Solid slots 1..=9 and Multi slots 1..=4 (from the classic
/// `stairs.txt` columns). The other tile types and lists have no established per-slot meaning.
pub fn housing_slot_label(tile_type: u32, list: HousingSlotList, slot: u32) -> Option<HousingSlotLabel> {
    let labels: &[&'static str] = match (tile_type, list) {
        (HOUSING_TILE_TYPE_STAIRS, HousingSlotList::Solid) => &STAIRS_SOLID_SLOT_LABELS,
        (HOUSING_TILE_TYPE_STAIRS, HousingSlotList::Multi) => &STAIRS_MULTI_SLOT_LABELS,
        _ => return None,
    };
    let index = usize::try_from(slot.checked_sub(FIRST_LABELLED_SLOT)?).ok()?;
    labels.get(index).map(|&label| HousingSlotLabel { label, confidence: HousingConfidence::Medium })
}

/// Lowercase name of a `HOUSING_TILE_TYPE_*` value.
pub fn housing_tile_type_name(tile_type: u32) -> Option<&'static str> {
    match tile_type {
        HOUSING_TILE_TYPE_STAIRS => Some("stairs"),
        HOUSING_TILE_TYPE_ROOF => Some("roof"),
        HOUSING_TILE_TYPE_DOOR => Some("door"),
        HOUSING_TILE_TYPE_FLOOR => Some("floor"),
        HOUSING_TILE_TYPE_WALL => Some("wall"),
        HOUSING_TILE_TYPE_TELEPORTER => Some("teleporter"),
        HOUSING_TILE_TYPE_MISC => Some("misc"),
        _ => None,
    }
}

/// Tile type name, or `tile type N` for an unknown tile type.
struct TileTypeLabel(u32);

impl fmt::Display for TileTypeLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match housing_tile_type_name(self.0) {
            Some(name) => f.write_str(name),
            None => write!(f, "tile type {}", self.0),
        }
    }
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_count(bytes: &mut Vec<u8>, count: usize) {
    push_u32(bytes, u32::try_from(count).expect("housing.bin counts are u32"));
}

fn read_piece(reader: &mut Cursor<&[u8]>) -> eyre::Result<HousingPiece> {
    let category = read_u32(reader, "piece category")?;
    let group = read_u32(reader, "piece group")?;
    let feature_mask = read_u32(reader, "piece feature mask")?;
    let tooltip_cliloc = read_u32(reader, "piece tooltip cliloc")?;
    let solid = read_slots(reader, "solid")?;
    let multis = read_slots(reader, "multi")?;
    let windows = read_slots(reader, "window")?;
    Ok(HousingPiece {
        category,
        group,
        feature_mask,
        tooltip_cliloc,
        solid,
        multis,
        windows,
    })
}

fn read_slots(reader: &mut Cursor<&[u8]>, list: &str) -> eyre::Result<Vec<HousingSlot>> {
    let count = read_u32(reader, list)? as usize;
    let mut slots = Vec::with_capacity(count.min(remaining_len(reader) / SLOT_SIZE));
    for _ in 0..count {
        let slot = read_u32(reader, list)?;
        let id = read_u32(reader, list)?;
        slots.push(HousingSlot { slot, id });
    }
    Ok(slots)
}

fn read_u32(reader: &mut Cursor<&[u8]>, field: &str) -> eyre::Result<u32> {
    let position = reader.position();
    reader
        .read_u32::<LittleEndian>()
        .wrap_err_with(|| format!("truncated {HOUSING_PATH}: {field} at byte {position}"))
}

fn remaining_len(reader: &Cursor<&[u8]>) -> usize {
    reader.get_ref().len().saturating_sub(reader.position() as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::WriteBytesExt;

    const STAIRS_CATEGORY: u32 = 3;
    const WALL_CATEGORY: u32 = 7;
    const WALL_GROUP: u32 = 2;
    const STAIRS_TOOLTIP: u32 = 1021826;
    const WALL_TOOLTIP: u32 = 1156116;
    const STAIRS_SOLID: [HousingSlot; 2] = [HousingSlot { slot: 1, id: 0x071F }, HousingSlot { slot: 9, id: 0x0721 }];
    const STAIRS_MULTIS: [HousingSlot; 1] = [HousingSlot { slot: 1, id: 7600 }];
    const WALL_SOLID: [HousingSlot; 1] = [HousingSlot { slot: 1, id: 0x0009 }];
    const WALL_WINDOWS: [HousingSlot; 2] = [HousingSlot { slot: 1, id: 0x0010 }, HousingSlot { slot: 6, id: 0x0011 }];
    /// A feature mask bit without a known name.
    const UNNAMED_FEATURE_BIT: u32 = 0x2;

    fn write_slots(bytes: &mut Vec<u8>, slots: &[HousingSlot]) {
        bytes.write_u32::<LittleEndian>(slots.len() as u32).unwrap();
        for slot in slots {
            bytes.write_u32::<LittleEndian>(slot.slot).unwrap();
            bytes.write_u32::<LittleEndian>(slot.id).unwrap();
        }
    }

    fn write_piece(bytes: &mut Vec<u8>, piece: &HousingPiece) {
        for value in [piece.category, piece.group, piece.feature_mask, piece.tooltip_cliloc] {
            bytes.write_u32::<LittleEndian>(value).unwrap();
        }
        write_slots(bytes, &piece.solid);
        write_slots(bytes, &piece.multis);
        write_slots(bytes, &piece.windows);
    }

    fn sample() -> HousingPieces {
        HousingPieces {
            tile_types: vec![
                HousingTileType {
                    tile_type: HOUSING_TILE_TYPE_STAIRS,
                    pieces: vec![HousingPiece {
                        category: STAIRS_CATEGORY,
                        group: 0,
                        feature_mask: HOUSING_FEATURE_NO_EXPANSION,
                        tooltip_cliloc: STAIRS_TOOLTIP,
                        solid: STAIRS_SOLID.to_vec(),
                        multis: STAIRS_MULTIS.to_vec(),
                        windows: Vec::new(),
                    }],
                },
                HousingTileType {
                    tile_type: HOUSING_TILE_TYPE_WALL,
                    pieces: vec![HousingPiece {
                        category: WALL_CATEGORY,
                        group: WALL_GROUP,
                        feature_mask: HOUSING_FEATURE_TIME_OF_LEGENDS,
                        tooltip_cliloc: WALL_TOOLTIP,
                        solid: WALL_SOLID.to_vec(),
                        multis: Vec::new(),
                        windows: WALL_WINDOWS.to_vec(),
                    }],
                },
            ],
        }
    }

    fn encode(pieces: &HousingPieces) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.write_u32::<LittleEndian>(pieces.tile_types.len() as u32).unwrap();
        for tile_type in &pieces.tile_types {
            bytes.write_u32::<LittleEndian>(tile_type.tile_type).unwrap();
            bytes.write_u32::<LittleEndian>(tile_type.pieces.len() as u32).unwrap();
            for piece in &tile_type.pieces {
                write_piece(&mut bytes, piece);
            }
        }
        bytes
    }

    #[test]
    fn parses_all_three_slot_lists_in_file_order() {
        let expected = sample();

        let parsed = HousingPieces::from_payload_bytes(&encode(&expected)).unwrap();

        assert_eq!(parsed, expected);
        let stairs = &parsed.tile_types[0].pieces[0];
        assert_eq!(stairs.solid, STAIRS_SOLID);
        assert_eq!(stairs.multis, STAIRS_MULTIS);
        assert!(stairs.windows.is_empty());
        let wall = &parsed.tile_types[1].pieces[0];
        assert_eq!(wall.group, WALL_GROUP);
        assert_eq!(wall.windows, WALL_WINDOWS);
        assert_eq!(parsed.pieces().count(), 2);
        assert_eq!(housing_tile_type_name(parsed.tile_types[1].tile_type), Some("wall"));
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut bytes = encode(&sample());
        bytes.push(0);

        let error = HousingPieces::from_payload_bytes(&bytes).unwrap_err();
        assert!(error.to_string().contains("trailing"), "{error}");
    }

    #[test]
    fn rejects_truncated_payload() {
        let mut bytes = encode(&sample());
        bytes.truncate(bytes.len() - SLOT_SIZE);

        assert!(HousingPieces::from_payload_bytes(&bytes).is_err());
    }

    #[test]
    fn empty_payload_needs_the_tile_type_count() {
        assert!(HousingPieces::from_payload_bytes(&[]).is_err());
        let parsed = HousingPieces::from_payload_bytes(&encode(&HousingPieces::default())).unwrap();
        assert!(parsed.tile_types.is_empty());
    }

    #[test]
    fn serializer_is_the_inverse_of_the_parser() {
        let pieces = sample();

        let bytes = pieces.to_payload_bytes();

        assert_eq!(bytes, encode(&pieces));
        assert_eq!(HousingPieces::from_payload_bytes(&bytes).unwrap(), pieces);
        assert_eq!(HousingPieces::default().to_payload_bytes(), encode(&HousingPieces::default()));
    }

    #[test]
    fn category_helpers_follow_the_client_rules() {
        let mut pieces = sample();
        let wall = &mut pieces.tile_types[1];
        let mut first_group = wall.pieces[0].clone();
        first_group.group = 0;
        first_group.feature_mask = HOUSING_FEATURE_NO_EXPANSION;
        first_group.tooltip_cliloc = 0;
        wall.pieces.insert(0, first_group);

        assert_eq!(
            pieces.category_feature_mask(HOUSING_TILE_TYPE_WALL, WALL_CATEGORY),
            Some(HOUSING_FEATURE_TIME_OF_LEGENDS)
        );
        assert_eq!(pieces.category_tooltip(HOUSING_TILE_TYPE_WALL, WALL_CATEGORY), Some(WALL_TOOLTIP));
        assert_eq!(
            pieces.category_entitlement(HOUSING_TILE_TYPE_WALL, WALL_CATEGORY),
            Some(HousingEntitlement {
                feature_mask: HOUSING_FEATURE_TIME_OF_LEGENDS,
                expansion: Some("Time of Legends"),
                cliloc: HOUSING_ENTITLEMENT_CLILOC_TIME_OF_LEGENDS,
            })
        );
        assert_eq!(pieces.category_feature_mask(HOUSING_TILE_TYPE_WALL, STAIRS_CATEGORY), None);

        // A repeated tile type replaces the earlier one.
        let mut replacement = pieces.tile_types[0].clone();
        replacement.pieces[0].tooltip_cliloc = WALL_TOOLTIP;
        pieces.tile_types.push(replacement);
        assert_eq!(pieces.category_tooltip(HOUSING_TILE_TYPE_STAIRS, STAIRS_CATEGORY), Some(WALL_TOOLTIP));
    }

    #[test]
    fn entitlement_cliloc_depends_on_the_exact_mask() {
        assert_eq!(housing_entitlement_cliloc(HOUSING_FEATURE_GOTHIC_THEME), HOUSING_ENTITLEMENT_CLILOC_GOTHIC_THEME);
        assert_eq!(housing_entitlement_cliloc(HOUSING_FEATURE_RUSTIC_THEME), HOUSING_ENTITLEMENT_CLILOC_RUSTIC_THEME);
        assert_eq!(housing_entitlement_cliloc(HOUSING_FEATURE_SA), HOUSING_ENTITLEMENT_CLILOC_UPGRADE);
        assert_eq!(
            housing_entitlement_cliloc(HOUSING_FEATURE_GOTHIC_THEME | HOUSING_FEATURE_SA),
            HOUSING_ENTITLEMENT_CLILOC_UPGRADE
        );
        assert_eq!(housing_entitlement(HOUSING_FEATURE_AOW).expansion, Some("AOW"));
        assert_eq!(housing_entitlement(HOUSING_FEATURE_AOW | HOUSING_FEATURE_SA).expansion, None);
        assert_eq!(housing_feature_name(HOUSING_FEATURE_EP1), Some("EP1"));
        assert_eq!(housing_feature_name(UNNAMED_FEATURE_BIT), None);
        assert!(HOUSING_FEATURES.iter().all(|(bit, _)| bit.is_power_of_two()));
    }

    #[test]
    fn tile_index_lists_solid_and_window_slots_by_tile_id() {
        let mut pieces = sample();
        pieces.tile_types[1].pieces[0].windows.push(HousingSlot { slot: 2, id: WALL_SOLID[0].id });

        let index = pieces.tile_index();

        let wall = HousingPieceKey { tile_type: HOUSING_TILE_TYPE_WALL, category: WALL_CATEGORY, group: WALL_GROUP };
        assert_eq!(
            index[&WALL_SOLID[0].id],
            [
                HousingSlotRef { piece: wall, list: HousingSlotList::Solid, slot: 1 },
                HousingSlotRef { piece: wall, list: HousingSlotList::Window, slot: 2 },
            ]
        );
        assert!(!index.contains_key(&STAIRS_MULTIS[0].id), "multi ids are not tile ids");
        let added_window = 1;
        assert_eq!(
            index.values().map(Vec::len).sum::<usize>(),
            STAIRS_SOLID.len() + WALL_SOLID.len() + WALL_WINDOWS.len() + added_window
        );
    }

    #[test]
    fn slot_labels_cover_the_stairs_lists() {
        let label = |list, slot| housing_slot_label(HOUSING_TILE_TYPE_STAIRS, list, slot).map(|label| label.label);

        assert_eq!(label(HousingSlotList::Solid, 1), Some("North"));
        assert_eq!(label(HousingSlotList::Solid, 9), Some("Block"));
        assert_eq!(label(HousingSlotList::Solid, 0), None);
        assert_eq!(label(HousingSlotList::Solid, 10), None);
        assert_eq!(label(HousingSlotList::Multi, 4), Some("MultiWest"));
        assert_eq!(
            housing_slot_label(HOUSING_TILE_TYPE_STAIRS, HousingSlotList::Multi, 1).map(|label| label.confidence),
            Some(HousingConfidence::Medium)
        );
        assert_eq!(housing_slot_label(HOUSING_TILE_TYPE_WALL, HousingSlotList::Solid, 1), None);
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("uocf_housing_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_and_loads_a_loose_housing_bin() {
        let dir = temp_dir("loose");
        let path = dir.join("housing.bin");

        sample().write_housing_bin(&path).unwrap();

        assert_eq!(HousingPieces::load_housing_bin(&path).unwrap(), sample());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writes_a_multi_collection_with_only_housing_replaced() {
        use crate::enhanced::multis::multi_collection_path;
        use crate::uop_container::file::CompressionFlag;

        const MULTI_PAYLOAD: &[u8] = b"multi payload";
        let dir = temp_dir("package");
        let source = dir.join("source.uop");
        let output = dir.join("output.uop");
        let mut package = UopPackage::new_default();
        let multi_path = multi_collection_path(STAIRS_MULTIS[0].id);
        let empty_housing = encode(&HousingPieces::default());
        package.add_file_from_memory(MULTI_PAYLOAD, &multi_path, CompressionFlag::None).unwrap();
        package.add_file_from_memory(&empty_housing, HOUSING_PATH, CompressionFlag::Zlib).unwrap();
        package.finalize_and_save(&source).unwrap();

        sample().write_multi_collection(&source, &output).unwrap();

        let written = UopPackage::load(&output).unwrap();
        assert_eq!(HousingPieces::from_package(&written).unwrap(), Some(sample()));
        let housing = written.get_file_by_name(HOUSING_PATH).unwrap();
        assert_eq!(housing.compression(), CompressionFlag::Zlib);
        let multi = written.get_file_by_name(&multi_path).unwrap();
        assert_eq!(multi.unpack().unwrap(), MULTI_PAYLOAD);
        assert_eq!(written.file_count(), package.file_count());

        let error = sample().write_multi_collection(&source, dir.join(".").join("source.uop")).unwrap_err();
        assert!(error.to_string().contains("refusing to overwrite"), "{error}");
        let source_housing = UopPackage::load(&source).unwrap().get_file_by_name(HOUSING_PATH).unwrap().unpack().unwrap();
        assert_eq!(source_housing, empty_housing, "the source is left unchanged");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replacing_needs_a_housing_entry() {
        let mut package = UopPackage::new_default();

        assert!(sample().replace_in_package(&mut package).is_err());
    }
}
