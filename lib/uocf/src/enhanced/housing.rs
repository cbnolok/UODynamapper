//! Decoding for `build/multicollection/housing.bin` in `MultiCollection.uop`: the piece tables of the house
//! customization tool.
//!
//! Layout and field meanings follow the client parser (`UOCustomHousingPieces::Parse` in UOSA.exe) and its
//! only reader (`UOHouseCustomization::BuildPieceTables`), documented in
//! `scripts/dev-helpers/ec-research/ghidra-housing/results/uosa_housing.md` (sections 2 and 3). The asset is
//! separate from the multi entries of the same package; the only link is the stairs multi list, which refers to
//! the stair block multis.

use std::io::Cursor;
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

// Entitlement clilocs (`entitlementTID`) the client picks from the feature mask of a category (the mask of its
// last piece). Confidence: high.

/// "You must upgrade to the latest Expansion Pack ...": every mask except the theme packs and Time of Legends.
pub const HOUSING_ENTITLEMENT_CLILOC_UPGRADE: u32 = 1062930;
pub const HOUSING_ENTITLEMENT_CLILOC_GOTHIC_THEME: u32 = 1150774;
pub const HOUSING_ENTITLEMENT_CLILOC_RUSTIC_THEME: u32 = 1150773;
pub const HOUSING_ENTITLEMENT_CLILOC_TIME_OF_LEGENDS: u32 = 1156114;

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

    pub fn pieces(&self) -> impl Iterator<Item = (u32, &HousingPiece)> {
        self.tile_types
            .iter()
            .flat_map(|tile_type| tile_type.pieces.iter().map(|piece| (tile_type.tile_type, piece)))
    }
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
}
