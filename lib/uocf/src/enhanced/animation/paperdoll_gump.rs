//! The Enhanced Client's old-style gump paperdoll: the number a worn item's gump is numbered by, as
//! `UOPaperdoll::BuildImage` (UOSA 0x0050c790) picks it. The rule, its addresses and the rest of what the client
//! draws are in `docs/client_research/ghidra-ec-paperdoll-gump/results/uosa_paperdoll_gump.md`.
//!
//! The records are read through [`PaperdollDefinitions`], so tests and dynamapper (over `tilemeta.uddp`) run the same
//! rule.

use crate::enhanced::mobile_body::MOBILE_DEFINITION_BASE;
use crate::enhanced::paperdoll::{PaperdollDefinition, PaperdollDefinitions};
use crate::enhanced::tileart::PropertyKey;

/// The `prop_vector2` keys `BuildImage` numbers an item by, in its order: Paperdoll (0x0050cb29), then Appearance
/// (0x0050cb73).
const NUMBER_KEYS: [PropertyKey; 2] = [PropertyKey::Paperdoll, PropertyKey::Appearance];

/// The number of the gump item `item_id` draws. It is `prop_vector2` Paperdoll, else `prop_vector2` Appearance, of
/// the item's record; else, also without a record, the item's own id. A value above 10,000,000 loses it (0x0050cbbc).
/// The client draws gump number + 50,000, or for a female body number + 60,000 when it exists.
///
/// `prop_vector1`, the appearance filters and tiledata take no part. Neither does `equipconv.csv`, which could replace
/// the number per body: it ships in no package. The client draws nothing when the record a value names (value +
/// 10,000,000) is missing; this returns the number all the same.
pub fn legacy_gump_number<'a, S: PaperdollDefinitions<'a>>(definitions: S, item_id: u32) -> u32 {
    let Some(definition) = definitions.get(item_id) else {
        return item_id;
    };
    NUMBER_KEYS.iter().find_map(|&key| definition.property2(key)).map_or(item_id, |value| {
        if value > MOBILE_DEFINITION_BASE { value - MOBILE_DEFINITION_BASE } else { value }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enhanced::paperdoll::SortedTileArt;
    use crate::enhanced::tileart::{TaeProp, TileArtEntry};

    const SHIRT: u32 = 5399;
    const SHIRT_APPEARANCE: u32 = 434;
    const CLOAK: u32 = 5397;
    const CLOAK_PAPERDOLL: u32 = 885;
    const CLOAK_APPEARANCE: u32 = 420;
    const RING: u32 = 4234;
    const RING_APPEARANCE: u32 = 511;
    const SHIELD: u32 = 7110;
    const UNRECORDED: u32 = 9594;
    const PLACEHOLDER_VALUE: u32 = MOBILE_DEFINITION_BASE + SHIRT_APPEARANCE;

    fn props(properties: &[(PropertyKey, u32)]) -> Vec<TaeProp> {
        properties.iter().map(|&(key, val)| TaeProp { id: key as u8, val }).collect()
    }

    fn record(tile_id: u32, props1: &[(PropertyKey, u32)], props2: &[(PropertyKey, u32)]) -> TileArtEntry {
        TileArtEntry { tile_id, prop_vector1: props(props1), prop_vector2: props(props2), ..TileArtEntry::default() }
    }

    /// Records sorted by tile id, as `SortedTileArt` finds them.
    fn records() -> Vec<TileArtEntry> {
        let mut records = vec![
            record(SHIRT, &[(PropertyKey::Appearance, SHIRT_APPEARANCE)], &[(PropertyKey::Appearance, SHIRT_APPEARANCE)]),
            record(
                CLOAK,
                &[(PropertyKey::Appearance, CLOAK_APPEARANCE)],
                &[(PropertyKey::Appearance, CLOAK_APPEARANCE), (PropertyKey::Paperdoll, CLOAK_PAPERDOLL)],
            ),
            // props1 Appearance alone: BuildImage does not read it.
            record(RING, &[(PropertyKey::Appearance, RING_APPEARANCE)], &[]),
            record(SHIELD, &[], &[(PropertyKey::Appearance, PLACEHOLDER_VALUE)]),
        ];
        records.sort_by_key(|record| record.tile_id);
        records
    }

    #[test]
    fn props2_paperdoll_comes_before_props2_appearance() {
        let records = records();
        let definitions = SortedTileArt(&records);
        assert_eq!(legacy_gump_number(definitions, SHIRT), SHIRT_APPEARANCE);
        assert_eq!(legacy_gump_number(definitions, CLOAK), CLOAK_PAPERDOLL);
    }

    #[test]
    fn without_props2_the_item_is_numbered_by_its_own_id() {
        let records = records();
        let definitions = SortedTileArt(&records);
        assert_eq!(legacy_gump_number(definitions, RING), RING, "props1 is not read");
        assert_eq!(legacy_gump_number(definitions, UNRECORDED), UNRECORDED, "no record");
    }

    #[test]
    fn a_value_above_the_definition_base_loses_it() {
        let records = records();
        assert_eq!(legacy_gump_number(SortedTileArt(&records), SHIELD), SHIRT_APPEARANCE);
    }
}
