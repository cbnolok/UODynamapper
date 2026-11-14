//! Multis: their definitions (`multi.mul` or `MultiCollection.uop`), and the multis placed in the world by their
//! multi id (`PlacedMultisRes`), which the renderer, the z-view limit and the light cover expand into parts.
//!
//! Map statics are never multis, whatever their graphic. `statics*.mul` (and the converted statics packages) hold
//! plain items, and since the High Seas and Stygian Abyss tiledata item ids run up to 0xFFFF, so the maps hold
//! ordinary items at 0x4000 and above (map 0: 3706, map 1: 9469, map 5: 168922). The emulators read the statics the
//! same way: ServUO's `TileMatrix.ReadStaticBlock` (Server/TileMatrix.cs) and ModernUO's read every entry as an item
//! with no 0x4000 test, keep the multis as separate world objects, and ModernUO's TileData.cs caps item ids at
//! `MaxItemValue` 0xFFFF. 0x4000 marked a multi only in the legacy 0x1A WorldItem packet, whose graphic field is an
//! item id cut to 14 bits or a multi id or'ed with 0x4000 (ServUO's `WorldItem`, Server/Network/Packets.cs; ClassicUO's
//! `PacketHandlers.UpdateItem`: `if (graphic >= 0x4000) type = 2`); the 0xF3 packet that ServUO and ModernUO send
//! instead carries a type byte, 2 for a multi. Dynamapper supports neither that convention nor the 0x1A packet, now or
//! later: 0x4000 divides nothing between items and multis here. A multi is drawn only from a source that names it as
//! one, through `PlacedMultisRes`.

use bevy::prelude::*;
use std::ops::Range;

pub const MULTI_PART_VISIBLE_FLAG: u32 = 0x1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MultiPart {
    pub item_id: u16,
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

/// Multi definitions indexed by multi id. Parts the client never draws (classic parts without
/// `MULTI_PART_VISIBLE_FLAG`, EC `not_drawn` parts) are kept apart, so they can be shown on demand.
#[derive(Resource, Default)]
pub struct MultiDefinitionsRes {
    visible_definitions: Vec<Vec<MultiPart>>,
    all_definitions: Vec<Vec<MultiPart>>,
}

impl MultiDefinitionsRes {
    pub fn from_classic_parts(parts: Vec<Vec<uocf::classic::multi::MultiPart>>) -> Self {
        Self::from_tagged_parts(parts.into_iter().map(|parts| {
            parts
                .into_iter()
                .map(|part| {
                    (
                        MultiPart {
                            item_id: part.item_id,
                            x: part.x,
                            y: part.y,
                            z: part.z,
                        },
                        multi_part_is_visible(part.flags),
                    )
                })
                .collect()
        }))
    }

    pub fn from_ec_collection(collection: &uocf::enhanced::multis::MultiCollection) -> Self {
        Self::from_ec_items(&collection.items)
    }

    fn from_ec_items(items: &[uocf::enhanced::multis::MultiItem]) -> Self {
        let max_id = items.iter().map(|item| item.id).max().unwrap_or(0);
        let mut definitions = vec![Vec::new(); max_id as usize + 1];

        for item in items {
            definitions[item.id as usize] = item
                .parts
                .iter()
                .map(|part| {
                    (
                        MultiPart {
                            item_id: part.item_id,
                            x: part.x,
                            y: part.y,
                            z: part.z,
                        },
                        !part.not_drawn,
                    )
                })
                .collect();
        }

        Self::from_tagged_parts(definitions)
    }

    /// Builds both definition lists from parts tagged with whether the client draws them.
    fn from_tagged_parts(definitions: impl IntoIterator<Item = Vec<(MultiPart, bool)>>) -> Self {
        let mut visible_definitions = Vec::new();
        let mut all_definitions = Vec::new();
        for parts in definitions {
            visible_definitions.push(
                parts
                    .iter()
                    .filter(|(_, visible)| *visible)
                    .map(|(part, _)| *part)
                    .collect(),
            );
            all_definitions.push(parts.into_iter().map(|(part, _)| part).collect());
        }

        Self {
            visible_definitions,
            all_definitions,
        }
    }

    /// Parts of a multi, optionally including the ones the client never draws.
    pub fn parts(&self, multi_id: u32, include_hidden: bool) -> Option<&[MultiPart]> {
        let definitions = if include_hidden {
            &self.all_definitions
        } else {
            &self.visible_definitions
        };
        definitions
            .get(multi_id as usize)
            .map(Vec::as_slice)
            .filter(|parts| !parts.is_empty())
    }

    pub fn definition_count(&self) -> usize {
        self.visible_definitions
            .iter()
            .filter(|parts| !parts.is_empty())
            .count()
    }
}

/// A multi placed in the world by its multi id, anchored on the cell (`x`, `y`) at `z`: its parts lie at their offsets
/// from the anchor, their z added to the anchor's (`expanded_multi_part_z`), all with the multi's hue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlacedMulti {
    pub multi_id: u32,
    pub x: u32,
    pub y: u32,
    pub z: i8,
    pub hue: u16,
}

/// The multis placed in the world, by map. Only a source that names its objects as multis may fill it, such as a
/// server's 0xF3 objects of type 2 or a placement file, never the map statics (see the module comment). None does
/// yet, so it stays empty and no multi is drawn. A source that changes it must also invalidate what was built from
/// it: the static chunk cache, the z-view limit and the light scans.
#[derive(Resource, Default)]
pub struct PlacedMultisRes {
    maps: Vec<Vec<PlacedMulti>>,
}

impl PlacedMultisRes {
    /// The multis placed on `map_id` whose anchors lie in the cells `x` × `y`.
    pub fn anchored_in(&self, map_id: u32, x: Range<u32>, y: Range<u32>) -> impl Iterator<Item = &PlacedMulti> {
        self.maps
            .get(map_id as usize)
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .filter(move |multi| x.contains(&multi.x) && y.contains(&multi.y))
    }

    #[cfg(test)]
    pub fn from_maps(maps: Vec<Vec<PlacedMulti>>) -> Self {
        Self { maps }
    }
}

fn multi_part_is_visible(flags: u32) -> bool {
    flags & MULTI_PART_VISIBLE_FLAG != 0
}

pub fn expanded_multi_part_z(base_z: i8, part_z: i16) -> i8 {
    (base_z as i16 + part_z).clamp(i8::MIN as i16, i8::MAX as i16) as i8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placed_multis_are_found_by_map_and_anchor() {
        let multi = |multi_id, x, y| PlacedMulti { multi_id, x, y, z: 0, hue: 0 };
        let placed = PlacedMultisRes::from_maps(vec![Vec::new(), vec![multi(1, 10, 10), multi(2, 20, 10)]]);
        let ids = |map_id, x, y| placed.anchored_in(map_id, x, y).map(|multi| multi.multi_id).collect::<Vec<_>>();
        assert_eq!(ids(1, 0..15, 0..15), [1]);
        assert_eq!(ids(1, 0..21, 10..11), [1, 2]);
        assert!(ids(0, 0..21, 0..21).is_empty());
        assert!(ids(5, 0..21, 0..21).is_empty());
        assert!(PlacedMultisRes::default().anchored_in(0, 0..u32::MAX, 0..u32::MAX).next().is_none());
    }

    #[test]
    fn classic_conversion_keeps_only_visible_parts() {
        let definitions = MultiDefinitionsRes::from_classic_parts(vec![vec![
            uocf::classic::multi::MultiPart {
                item_id: 1,
                x: 0,
                y: 0,
                z: 0,
                flags: MULTI_PART_VISIBLE_FLAG,
            },
            uocf::classic::multi::MultiPart {
                item_id: 2,
                x: 0,
                y: 0,
                z: 0,
                flags: 0,
            },
        ]]);

        let parts = definitions.parts(0, false).expect("visible multi definition");
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].item_id, 1);

        let parts = definitions.parts(0, true).expect("full multi definition");
        assert_eq!(parts.len(), 2);
    }

    #[test]
    fn ec_conversion_keeps_only_drawn_parts_by_default() {
        let part = |item_id, not_drawn| uocf::enhanced::multis::MultiItemPart {
            item_id,
            x: 0,
            y: 0,
            z: 0,
            not_drawn,
            flag2: 0,
            string_ids: Vec::new(),
        };
        let definitions = MultiDefinitionsRes::from_ec_items(&[uocf::enhanced::multis::MultiItem {
            id: 1,
            filename_hash: 0,
            path: String::new(),
            parts: vec![part(1, false), part(2, true)],
        }]);

        assert_eq!(definitions.parts(0, false), None);
        let parts = definitions.parts(1, false).expect("drawn multi definition");
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].item_id, 1);

        let parts = definitions.parts(1, true).expect("full multi definition");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1].item_id, 2);
    }

    #[test]
    fn expanded_z_clamps_to_static_tile_range() {
        assert_eq!(expanded_multi_part_z(120, 20), i8::MAX);
        assert_eq!(expanded_multi_part_z(-120, -20), i8::MIN);
        assert_eq!(expanded_multi_part_z(10, -3), 7);
    }
}
